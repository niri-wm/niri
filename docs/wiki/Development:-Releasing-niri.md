This is a checklist of things to release a new niri version.

We'll use `26.04` as the example new version.
When making a patch release, append the patch number like `26.04.1`.

## Prepare the release notes

Plan for a few days of work, this usually takes a while.

During this process, also check:

- that all additions are marked with "next release" on the wiki,
- if anything needs updating in `README.md`.

## Bump version

We use `year.month.patch` versioning.
If the month contains a leading zero, drop it from the crate version (Cargo requirement).

You can use the command from [cargo-edit](https://github.com/killercup/cargo-edit):

```
cargo set-version 26.4.0
```

Then, manually update version in:

- `[package.metadata.generate-rpm]` in Cargo.toml
- Dependency example in `niri-ipc/README.md`
- Dependency example in `niri-ipc/src/lib.rs`

Do a full text search for the old version to make sure there are no other places.

## Replace all "Since: next release" mentions

Do a full text search for `next release`, replace everything with the new version number.

## Build, test, push, and have the CI run

Run all tests:

```
RUN_SLOW_TESTS=1 cargo test --release --all
```

- Run `cargo package -p niri-ipc` and make sure it succeeds.
- Make sure the CI passes.
- Make sure the niri-git COPR build passes.

## Trigger the "Prepare release" workflow on GitHub Actions

Set the "Public version" input to a version like `26.04`.

This workflow will:

- do some pre-release checks like grepping the wiki for "next version",
- make a vendored dependency archive,
- build and test niri with that dependency archive,
- draft a new GitHub release with the archive attached.
It will NOT override an existing draft release with the same name so the release notes are safe.

Make sure it succeeds and grab the vendored dependency archive that it produces.

## Update the niri COPR spec, update licenses in .spec.rpkg

You can grab the previous spec from [the last build](https://copr.fedorainfracloud.org/coprs/yalter/niri/builds/) in the COPR.

- Update version global to `26.04`.
- Update commit global to the commit hash corresponding to the release commit.
You can use `git rev-parse HEAD`.
- Reset the `Release:` number to 1 if it was higher.

To run a test build, you can download the vendored dependency archive from the last step.
Comment/uncomment `Source:` and `%autosetup` lines accordingly.

Download the source files:

```
spectool -g niri.spec
```

Build RPMs:

```
fedpkg --release 44 mockbuild
```

During the build, it will print the list of licenses.
Update it in both the COPR spec and in `niri.spec.rpkg` accordingly.

If you had to update `niri.spec.rpkg` and therefore make another commit to the niri repo, make sure to update the commit hash in the COPR spec again.

Revert any temporary changes that you did to the COPR spec for local testing.

## Create and push the release git tag

The tag starts with a `v`:

```
git tag -am "v26.04 release" v26.04
git push origin v26.04
```

While you can let GitHub create the tag automatically upon creating the release, this is not recommended.
GitHub creates a *lightweight* tag, but we want an annotated tag that plays better with various tooling.

## Publish the release on GitHub

- Either upload the vendored dependencies file to your draft release with the release notes, or move the release notes to the GitHub-created release (the difference is that it's attributed to github-actions).
- Set the tag to `v26.04`.
- Set the release title to `v26.04`.
- Check "Create a discussion for this release".

## Publish the niri-ipc crate

```
cargo publish -p niri-ipc
```

## Kick off the COPR build

Upload on the web or:

```
copr-cli build niri niri.spec
```

## Announce the release

Chat rooms, social media, etc.
