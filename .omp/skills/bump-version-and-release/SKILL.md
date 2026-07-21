---
name: bump-version-and-release
description: Bump the app version, update CHANGELOG and README, create a git tag, push, and create a GitHub release with release notes.
---

# Bump Version and Release

## Version locations

Bump the version string in **all 3 files** (keep them in sync):

| File | Field | Format | Notes |
|------|-------|--------|-------|
| `package.json` | `"version"` | `"0.X.0b"` | App version, semver + `b` suffix for beta |
| `src-tauri/tauri.conf.json` | `"version"` | `"0.X.0"` | Tauri app version (no `b` suffix) |
| `src-tauri/Cargo.toml` | `version` | `"0.X.0"` | Rust crate version (no `b` suffix) |

**Do NOT touch** `src/utils/projectIO.ts` `CURRENT_VERSION` — that is the **project file format** version, only bumped when serialization schema changes (new/removed fields in `ShaderNodeData` or `ProjectFile`).

## Steps

### 1. Determine new version

- Check current version: `grep '"version"' package.json`
- Check recent tags: `git tag --sort=-creatordate | head -5`
- Increment minor version (e.g. `0.12.0b` -> `0.13.0b`)

### 2. Bump version in all 3 files

```
package.json          →  "version": "0.X.0b"
src-tauri/Cargo.toml  →  version = "0.X.0"
src-tauri/tauri.conf.json → "version": "0.X.0"
```

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
- Test count in the `npm test` line (e.g. `# 1045 unit tests`)
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
  --prerelease
```

- Use `--prerelease` for beta versions (suffix `b`)
- Release notes use GitHub Markdown (## headings, bold, backticks)
- Include a Stats section with test count

## Checklist

- [ ] Version bumped in `package.json`, `Cargo.toml`, `tauri.conf.json`
- [ ] CHANGELOG.md updated with new section
- [ ] README.md reviewed and updated (test count, features, roadmap)
- [ ] All tests pass (`npm test`)
- [ ] Type check passes (`npx tsc --noEmit`)
- [ ] Committed with `release: v0.X.0b` message
- [ ] Annotated tag `v0.X.0b` created
- [ ] Pushed to origin with tags
- [ ] GitHub release created with release notes
