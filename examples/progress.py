"""Poll native history traversal and cancel it from the controlling thread."""

import argparse
from concurrent.futures import ThreadPoolExecutor, TimeoutError

import gix


def main(path, limit):
    repo = gix.discover(path)
    progress = gix.Progress()
    cancel = gix.CancellationToken()

    def walk():
        count = 0
        with repo.rev_walk(["HEAD"]).all(progress=progress, cancel=cancel) as commits:
            for _ in commits:
                count += 1
                if count >= limit:
                    cancel.cancel()
        return count

    with ThreadPoolExecutor(max_workers=1) as executor:
        result = executor.submit(walk)
        try:
            while True:
                try:
                    print("Commits:", result.result(timeout=0.1))
                    break
                except TimeoutError:
                    print(progress.snapshot())
                except gix.CancelledError:
                    print("Traversal cancelled")
                    break
        except KeyboardInterrupt:
            cancel.cancel()
            raise
    print(progress.state)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", default=".")
    parser.add_argument("--limit", type=int, default=1000)
    args = parser.parse_args()
    main(args.path, args.limit)
