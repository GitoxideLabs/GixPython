"""Check shipped stubs against the selected native build without typing dependencies."""

import ast
import inspect
from pathlib import Path
import unittest

import gix

PROTOCOL_METHODS = {"__bytes__", "__iter__", "__next__", "__enter__", "__exit__", "__len__", "__getitem__", "__setitem__", "__contains__"}


def stub_api():
    classes = {}
    aliases = {}
    functions = {}
    exports = set()
    for path in sorted(Path(gix.__file__).parent.glob("*.pyi")):
        module = ast.parse(path.read_text(), filename=str(path))
        for node in module.body:
            if isinstance(node, ast.ClassDef):
                classes[node.name] = node
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                functions[node.name] = node
            elif isinstance(node, ast.ImportFrom):
                for alias in node.names:
                    aliases[alias.asname or alias.name] = alias.name
            if path.name != "_gix.pyi":
                continue
            if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AnnAssign)):
                exports.add(node.target.id if isinstance(node, ast.AnnAssign) else node.name)
            elif isinstance(node, ast.ImportFrom):
                # Stub imports re-export a public name only with an explicit alias.
                exports.update(alias.name for alias in node.names if alias.asname == alias.name)
    return classes, aliases, functions, exports


def class_members(name, classes, aliases):
    name = aliases.get(name, name)
    node = classes.get(name)
    if node is None:
        return {}
    members = {}
    for base in node.bases:
        if isinstance(base, ast.Name):
            members.update(class_members(base.id, classes, aliases))
    for member in node.body:
        if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef)):
            members[member.name] = member
        elif isinstance(member, ast.AnnAssign) and isinstance(member.target, ast.Name):
            members[member.target.id] = member
    return members


def stub_parameters(node):
    positional = node.args.posonlyargs + node.args.args
    required_count = len(positional) - len(node.args.defaults)
    result = []
    for index, arg in enumerate(positional):
        if arg.arg in ("self", "cls") and index == 0:
            continue
        kind = inspect.Parameter.POSITIONAL_ONLY if index < len(node.args.posonlyargs) else inspect.Parameter.POSITIONAL_OR_KEYWORD
        result.append((arg.arg, kind, index >= required_count))
    if node.args.vararg:
        result.append((node.args.vararg.arg, inspect.Parameter.VAR_POSITIONAL, False))
    result.extend((arg.arg, inspect.Parameter.KEYWORD_ONLY, default is not None) for arg, default in zip(node.args.kwonlyargs, node.args.kw_defaults))
    if node.args.kwarg:
        result.append((node.args.kwarg.arg, inspect.Parameter.VAR_KEYWORD, False))
    return result


def native_parameters(value):
    result = []
    for index, parameter in enumerate(inspect.signature(value).parameters.values()):
        if parameter.name in ("self", "cls") and index == 0:
            continue
        result.append((parameter.name, parameter.kind, parameter.default is not inspect.Parameter.empty))
    return result


def scalar_defaults(node):
    positional = node.args.posonlyargs + node.args.args
    defaults = zip(positional[len(positional) - len(node.args.defaults):], node.args.defaults)
    for arg, default in list(defaults) + list(zip(node.args.kwonlyargs, node.args.kw_defaults)):
        if default is None:
            continue
        try:
            value = ast.literal_eval(default)
        except (ValueError, TypeError):
            continue
        if value is None or isinstance(value, (str, bytes, bool, int, float)):
            yield arg.arg, value


def declares_method(node):
    if not isinstance(node, ast.FunctionDef):
        return False
    return not any(
        isinstance(decorator, ast.Name) and decorator.id == "property"
        or isinstance(decorator, ast.Attribute) and decorator.attr in ("setter", "deleter")
        for decorator in node.decorator_list
    )


class TypingTests(unittest.TestCase):
    maxDiff = None

    def test_runtime_exports_and_members_have_stubs(self):
        classes, aliases, functions, exports = stub_api()
        missing = []
        for name in dir(gix):
            if name.startswith("_"):
                continue
            value = getattr(gix, name)
            if not callable(value):
                continue
            if name not in exports:
                missing.append(name)
            if not isinstance(value, type) or issubclass(value, BaseException):
                continue
            members = class_members(name, classes, aliases)
            for member in vars(value):
                if (not member.startswith("_") or member in PROTOCOL_METHODS) and member not in members:
                    missing.append(f"{name}.{member}")
                elif not member.startswith("_") and member in members:
                    if callable(getattr(value, member)) != declares_method(members[member]):
                        missing.append(f"{name}.{member}: method/property mismatch")
        self.assertEqual(missing, [], "Native exports/members missing from shipped stubs")

    def test_native_parameter_names_kinds_and_defaults_match_stubs(self):
        classes, aliases, functions, _ = stub_api()
        mismatches = []

        def check(label, value, node):
            try:
                actual = native_parameters(value)
            except (TypeError, ValueError):
                return
            expected = stub_parameters(node)
            if actual != expected:
                mismatches.append(f"{label}: runtime={actual}, stub={expected}")
            signature = inspect.signature(value)
            for name, expected_default in scalar_defaults(node):
                parameter = signature.parameters.get(name)
                if parameter is None:
                    continue
                actual_default = parameter.default
                if actual_default is None or isinstance(actual_default, (str, bytes, bool, int, float)):
                    if actual_default != expected_default:
                        mismatches.append(f"{label}.{name}: runtime default={actual_default!r}, stub={expected_default!r}")

        for name in dir(gix):
            if name.startswith("_"):
                continue
            value = getattr(gix, name)
            if isinstance(value, type):
                if issubclass(value, BaseException):
                    continue
                members = class_members(name, classes, aliases)
                if "__new__" in vars(value) and "__init__" in members:
                    check(name, value, members["__init__"])
                for member, node in members.items():
                    if member.startswith("_") or not isinstance(node, ast.FunctionDef):
                        continue
                    native = getattr(value, member, None)
                    if native is not None and callable(native):
                        check(f"{name}.{member}", native, node)
            elif callable(value) and name in functions:
                check(name, value, functions[name])
        self.assertEqual(mismatches, [], "\n".join(mismatches))


if __name__ == "__main__":
    unittest.main()
