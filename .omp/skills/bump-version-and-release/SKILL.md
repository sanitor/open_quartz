---
name: bump-version-and-release
description: Bump the app version, update CHANGELOG and README, create a git tag, push, and create a GitHub release with release notes.
---

# Bump Version and Release

## Version locations

Bump the version string in **all 3 manifests** (keep them in sync):

| File | Field | Format | Notes |
|------|-------|--------|-------|
| `package.json` | `"version"` | `"0.X.0b"` | App version, semver + `b` suffix for beta |
| `apps/open_quartz_slint/Cargo.toml` | `version` | `"0.X.0"` | Native desktop version (no `b` suffix) |
| `crates/open_quartz/Cargo.toml` | `version` | `"0.X.0"` | Shared Rust SDK version (no `b` suffix) |

Update the version label in `apps/open_quartz_slint/ui/components/top_bar.slint` as well. The desktop packager derives the macOS bundle version from `package.json`. Keep the project file format version unchanged unless the serialization schema changes.

## Steps

### 1. Determine new version

- Check current version: `grep '"version"' package.json`
- Check recent tags: `git tag --sort=-creatordate | head -5`
- Increment minor version (e.g. `0.12.0b` -> `0.13.0b`)

### 2. Bump version in all 3 manifests

```
package.json          →  "version": "0.X.0b"
apps/open_quartz_slint/Cargo.toml → version = "0.X.0"
crates/open_quartz/Cargo.toml → version = "0.X.0"
```

Refresh `package-lock.json` with `npm install --package-lock-only --ignore-scripts` and refresh the native Cargo lockfiles without upgrading unrelated dependencies.

### 3. Update CHANGELOG.md

- Get commits since last tag: `git log v0.PREV.0b..HEAD --oneline`
- Insert a new section at the top (after `# Changelog`):

```markdown
## [0.X.0b] -- YYYY-MM-DD

### Features

- **Feature name** — description

### Fixes

- **Fix name** — description
```

- Group by Features / Fixes / Breaking Changes
- Each entry: bold name + em-dash + one-line description
- Match the style of existing entries

### 4. Update README.md

Review and update if needed:
- Native test counts and the commands that produced them
- Features section — move completed roadmap items to Features
- Roadmap section — remove completed items
- Node catalog — add new node types/shaders/models

### 5. Commit

```bash
git add -A
git commit -m "release: v0.X.0b

- bullet summary of key changes"
```

### 6. Create annotated tag

```bash
git tag -a v0.X.0b -m "v0.X.0b

Features:
- feature 1
- feature 2

Fixes:
- fix 1
- fix 2"
```

### 7. Push with tags

```bash
git push origin master --tags
```

### 8. Create GitHub release

```bash
gh release create v0.X.0b \
  --title "v0.X.0b" \
  --notes "## Features
- **Feature** — description

## Fixes
- **Fix** — description

## Stats
- NNNN unit tests, all passing
- NN test files" \
  --latest
```

- Do not pass `--prerelease`: GitHub cannot mark a prerelease as Latest, even when `--latest` is supplied; the `b` suffix remains the beta-version indicator
- Always pass `--latest` so the newly published version is explicitly marked as the repository's Latest release
- Release notes use GitHub Markdown (## headings, bold, backticks)
- Include a Stats section with test count

## Checklist

- [ ] Version bumped in `package.json`, `apps/open_quartz_slint/Cargo.toml`, and `crates/open_quartz/Cargo.toml`; toolbar label and native lockfiles updated
- [ ] CHANGELOG.md updated with new section
- [ ] README.md reviewed and updated (test count, features, roadmap)
- [ ] Native tests pass (`npm run test:desktop`)
- [ ] Desktop UI and shader suites pass (`npm run test:desktop:ui`, `npm run test:desktop:shaders`)
- [ ] Native package builds (`npm run build`)
- [ ] Committed with `release: v0.X.0b` message
- [ ] Annotated tag `v0.X.0b` created
- [ ] Pushed to origin with tags
- [ ] GitHub release created with release notes
- [ ] GitHub release is not a prerelease and is explicitly marked as Latest (`--latest`)
