# Contributing

Thank you for contributing to GixPython. Bug reports should include the Python version, platform, package version, enabled build features, and a small reproducible example. Where useful, describe Git's behavior for comparison.

Discuss substantial API or architectural changes before implementing them. The binding follows native `gix` API names and behavior; new Git behavior generally belongs upstream.

## Development

Read [DEVELOPMENT.md](DEVELOPMENT.md) and [agents.md](agents.md). Use isolated temporary repositories in tests, preserve byte-oriented data, and test relevant hash/build configurations. Include type information and documentation with API changes.

Each commit must be self-contained and pass the relevant checks. Use purposeful conventional commits, as Gitoxide does: `feat:` and `fix:` describe user-visible changes; ordinary maintenance uses a descriptive subject without a prefix. Breaking changes use `!` before the colon. Explain motivation and validation in the commit body.

## Agent disclosure

AI agents communicating through a person's account must identify themselves and speak for themselves. Do not claim that the account holder reviewed, tested, or approved the work. Editing someone's own prose does not make an agent its speaker.

Agent-created Git commits use explicit agent authorship. Sebastian Thiel remains the package author and copyright holder specified by this project. Review marks belong to the human reviewer.

## Security and publication

Follow [SECURITY.md](SECURITY.md) for vulnerabilities. Do not put sensitive reproductions in public issues.

The current implementation work is local only. Preparing workflow files or distribution artifacts does not authorize uploading, publishing, pushing, or opening a pull request. Artifact workflows deliberately contain no package-publishing or release-creation step.
