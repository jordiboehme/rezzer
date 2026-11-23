# Release notes

One file per version, named after its tag (for example `v0.1.0.md`). Each file starts with a level-one heading that holds the release motto. Next comes a one-line summary of the release. Then the sections New & Noteworthy, Fixes and Breaking changes. Write "Breaking changes: None." when there are none. The release workflow publishes the file as the release page, and a later change to the file on main updates that page.

## How a release runs

A merge to `main` makes the release. Nobody needs to push a tag.

1. The last pull request of a version changes `version` in `[workspace.package]` of `Cargo.toml`. It adds `release-notes/v<version>.md` and updates the version pins in `README.md`. CI checks these files with `scripts/ci/version-pins.sh`.
2. The `main` ruleset asks that the branch is up to date with `main` and that `ci-passed` is green before it merges. Admins can skip these rules. So the release does not trust them: CI runs again on the merge commit, and the release waits for the `ci-passed` of that run.
3. When that `ci-passed` is green, the `release-check` job runs `scripts/ci/release-check.sh`. A version that already has its tag and release does nothing. A new version without its notes file fails. While the release of an earlier commit is still being published, a later merge releases nothing and shows a warning.
4. For a new version with its notes, the `release` job calls the Release workflow with the tag `v<version>`. It builds, signs and checks everything. Then it creates the tag at the tested commit and publishes the release and the Homebrew formula.
5. If a release fails, re-run the failed jobs of that CI run. A merge that does not change the version releases nothing.

Pushing a `v*` tag by hand still releases. A manual run of the Release workflow is a dry run and publishes nothing. Run `bash scripts/ci/release-check.sh` to see what a merge of your branch would do.
