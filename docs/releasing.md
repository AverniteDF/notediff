# Releasing notediff

This is the release SOP for notediff. It is written for maintainers and AI
agents publishing a release. Remote operations — pushing, tagging and creating
a GitHub release — require explicit user approval.

## 1. Preflight

- Confirm the target version and release date with the user.
- Work from a clean tree on `main`; `git status --short` must be completely
  empty, including untracked files. Delete or commit scratch files such as a
  session handoff before starting:

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
  The `v1.0.0`, `v1.1.0`, `v1.2.0` and `v1.3.0` releases are source-only.
- Confirm the GitHub CLI is installed and authenticated before starting
  (section 6). The token needs the `repo` scope, and HTTPS pushes need
  `gh auth setup-git` when no credential helper is configured.
- Do not release from a dirty tree, a detached HEAD, or a branch that has not
  been reviewed and merged.

## 2. Version and date

Update all of these together:

- `Cargo.toml`: `version`.
- `Cargo.lock`: the root `notediff` package version. Running `cargo check`
  without `--locked` once after editing `Cargo.toml` updates it; commit the
  result. Confirm the root `name = "notediff"` entry shows the new version.
- `CHANGELOG.md`:
  - keep `## [Unreleased]` at the top, empty for the next cycle;
  - copy its entries under a new `## [X.Y.Z] - YYYY-MM-DD` heading beneath it;
  - keep the entries accurate and user-facing;
  - update the comparison links.
- `src/ui/about_dialog.rs`: set `RELEASE_DATE` to the release date in
  `Month D, YYYY` form. The dialog already renders `Released {RELEASE_DATE}.`,
  so no other About wording changes are needed.
- Any documentation that states the current version or release date.

## 3. Release notes

Begin the release body with a one-sentence synopsis of the release, then include
the new changelog section. The section must group its entries under
`### Added`, `### Changed` and `### Fixed` (Keep a Changelog order; omit empty
groups). Keep the release title in the form:

```text
notediff X.Y.Z
```

Do not invent features that are not in the changelog. The synopsis must match
the actual release.

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

Launch the installed build and confirm **About** shows the new version and
release date.

## 5. Commit, push and tag

Commit the release preparation with a message such as:

```sh
git add Cargo.toml Cargo.lock CHANGELOG.md src/ui/about_dialog.rs docs
git commit -m "Release notediff X.Y.Z"
```

Before pushing, confirm the release commit is `HEAD`, the tree is clean, and
the tag will point at it:

```sh
git status --short
git rev-parse HEAD
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
gh release view vX.Y.Z --json body -q .body > /tmp/notediff-X.Y.Z-body.md
diff -u /tmp/notediff-X.Y.Z-release.md /tmp/notediff-X.Y.Z-body.md
git ls-remote origin refs/heads/main refs/tags/vX.Y.Z 'refs/tags/vX.Y.Z^{}'
gh release list --limit 3
```

Check that:

- the release is public, latest, and not a draft or prerelease;
- the tag points at the release commit;
- `main` and the tag are present on `origin`;
- the release body matches the synopsis plus the changelog section. GitHub may
  add a trailing newline; compare the content, not the byte count.

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

## 10. Release checklist

- [ ] Target version and release date confirmed with the user.
- [ ] `git status --short` is empty, including untracked files.
- [ ] `gh` is installed and authenticated with the `repo` scope;
      `gh auth setup-git` is run when HTTPS pushes need it.
- [ ] `Cargo.toml` and `Cargo.lock` show `X.Y.Z`.
- [ ] `## [Unreleased]` is empty above `## [X.Y.Z] - YYYY-MM-DD`, and the
      comparison links point at the new tag.
- [ ] The changelog section groups entries under `### Added`, `### Changed`
      and `### Fixed`, and the release body opens with a matching synopsis.
- [ ] `RELEASE_DATE` is updated.
- [ ] fmt, clippy, unit tests, the GTK smoke test and the 1.92 check pass.
- [ ] The installed build's **About** shows the new version and date.
- [ ] The release commit is `HEAD`; the tree is clean.
- [ ] `main` and the annotated tag are pushed after approval and point at the
      same commit.
- [ ] The GitHub release is source-only unless assets were requested, is
      **Latest**, and its body matches the changelog.
- [ ] The release is public, not a draft or prerelease.
