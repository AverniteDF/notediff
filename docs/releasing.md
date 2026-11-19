# Releasing notediff

This is the release SOP for notediff. It is written for maintainers and AI
agents publishing a release. Remote operations — pushing, tagging and creating
a GitHub release — require explicit user approval.

## 1. Preflight

- Confirm the target version and release date with the user.
- Work from a clean tree on `main`:

```sh
git status --short --branch
git fetch origin
git log --oneline origin/main..HEAD
```

- Confirm the remote state directly when it matters:

```sh
git ls-remote --heads origin main
git ls-remote --tags origin 'v*'
```

- Confirm whether the release is source-only or should include built assets.
  The `v1.0.0` and `v1.1.0` releases are source-only.
- Do not release from a dirty tree, a detached HEAD, or a branch that has not
  been reviewed and merged.

## 2. Version and date

Update all of these together:

- `Cargo.toml`: `version`.
- `Cargo.lock`: the root `notediff` package version. Running `cargo check`
  without `--locked` once after editing `Cargo.toml` updates it; commit the
  result.
- `CHANGELOG.md`:
  - replace `[Unreleased]` with the release heading and date;
  - keep the entries accurate and user-facing;
  - update the comparison links.
- `src/ui/about_dialog.rs`: `RELEASE_DATE` and the **Released …** wording.
- Any documentation that states the current version or release date.

## 3. Release notes

Use the new changelog section as the GitHub release body. Keep the release title
in the form:

```text
notediff X.Y.Z
```

Do not invent features that are not in the changelog. A leading summary
sentence is optional, but it must match the actual release.

## 4. Verification

Run every release check:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
GDK_BACKEND=x11 NO_AT_BRIDGE=1 dbus-run-session -- \
  cargo test --locked gtk_window_smoke -- --ignored
cargo +1.92.0 check --locked
```

On a ChromeOS/Sommelier host, the explicit `GDK_BACKEND=x11` and
`NO_AT_BRIDGE=1` avoid the known Wayland-bridge display failure. CI also runs
the same checks on Debian trixie.

Optional local deployment check:

```sh
./install.sh
```

## 5. Commit, push and tag

Commit the release preparation with a message such as:

```sh
git add Cargo.toml Cargo.lock CHANGELOG.md src/ui/about_dialog.rs docs
git commit -m "Release notediff X.Y.Z"
```

Push `main` only after the user approves:

```sh
git push origin main
```

Create and push an annotated tag pointing at the release commit:

```sh
git tag -a vX.Y.Z -m "notediff X.Y.Z"
git push origin vX.Y.Z
```

Do not move or force-push an existing release tag without explicit approval.

## 6. GitHub authentication

Check whether `gh` is installed and authenticated:

```sh
command -v gh
gh auth status
```

If `gh` is missing, install it outside the repository. Do not commit the binary.

If authentication is needed, use the web/device flow:

```sh
gh auth login --hostname github.com --git-protocol https --web
```

Pause and give the user the one-time code. Never print, commit, or paste a
token into logs or project files. Prefer the system credential store. If the
environment falls back to plain-text storage, treat that file as a secret.

If git pushes need the credential helper:

```sh
gh auth setup-git
```

## 7. Create the GitHub release

Extract the release section from `CHANGELOG.md` (replace `X.Y.Z`), for example:

```sh
awk '/^## \[X\.Y\.Z\]/{flag=1; next} /^## \[/{flag=0} flag' \
  CHANGELOG.md > /tmp/notediff-X.Y.Z-release.md
```

Create the release after the tag is pushed:

```sh
gh release create vX.Y.Z \
  --title "notediff X.Y.Z" \
  --notes-file /tmp/notediff-X.Y.Z-release.md \
  --latest
```

For a source-only release, do not attach binaries. If the user requests assets,
build with the distribution profile in `docs/development.md` and provide
checksums and clear platform names.

Verify the result:

```sh
gh release view vX.Y.Z --json tagName,name,isDraft,isPrerelease,url,assets
```

Check that:

- the release is public, latest, and not a draft or prerelease;
- the tag points at the release commit;
- `main` and the tag are present on `origin`;
- the release body matches the changelog.

Optionally redeploy locally with `./install.sh`.

## 8. AI-agent rules

- Ask for explicit approval before each remote or destructive action:
  push, tag creation, tag push, release creation, deletion or retagging.
- Report the release commit, tag, URL and asset decisions when finished.
- Never place credentials in the repository, shell history, documentation or
  chat output.
- Do not guess the remote state: inspect `git status`, `git log`,
  `git ls-remote` and `gh auth status`.
- Do not publish a release while release checks are failing.
- Do not commit `target/`, downloaded tools, release-note scratch files or
  credentials.
- If the user asks for a hard reset or discarded work during a release, confirm
  that no committed release work will be lost.

## 9. Recovery

- If the release is wrong but the tag has not been used, fix the release notes
  and republish.
- If a tag or release must be removed, ask the user first. Prefer deleting the
  GitHub release before deleting the tag.
- Prefer a corrective commit and a new version over moving a public tag.
- Never force-push `main` by default.
