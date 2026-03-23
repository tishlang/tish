# How to Release Tish

Step-by-step guide. Do these in order.

---

## Before You Start: One-Time Setup

### 1. GitHub Secrets (Settings → Secrets and variables → Actions)

Add these to the **tish** repo:

| Secret | How to get it |
|--------|---------------|
| `NPM_TOKEN` | npmjs.com → Account → Access Tokens → Generate (Automation type, no expiry) |
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → New token |

Homebrew uses the tish repo as the tap (monorepo). No separate repo or token needed.

---

## Every Release

### Step 1: Commit with a release-triggering message

You need at least one commit that triggers a version bump. Use conventional commits:

```
feat: add something new        → minor (1.0.0 → 1.1.0)
fix: fix a bug                 → patch (1.0.0 → 1.0.1)
perf: make it faster           → patch
feat!: breaking change         → major (1.0.0 → 2.0.0)
```

`docs:` and `chore:` do **not** trigger a release. If CI fails with “No incremental release would be triggered”, you need a `feat`, `fix`, `perf`, or `BREAKING CHANGE` commit.

### Step 2: Push to `main`

```bash
git push origin main
```

### Step 3: Let CI run

- Open **Actions** in the tish repo
- Wait for the **CI (test, coverage, build)** workflow to finish
- The **Release (prerelease branch + GitHub API)** job must succeed

If it fails:

- **“No incremental release would be triggered”** → Add a `feat:`, `fix:`, or `perf:` commit and push again
- **Build/test failures** → Fix them and push again

### Step 4: Promote the prerelease to a full release

1. Go to **Releases** in the tish repo
2. Find the **latest prerelease** (e.g. `v1.0.13`)
3. Click **Edit**
4. **Uncheck** “Set as a pre-release”
5. Click **Update release**

This runs the NPM, Crates.io, and Homebrew release workflows. They run automatically; no further action needed.

---

## Verify

- **npm**: `npm view @tishlang/tish version`
- **crates.io**: https://crates.io/crates/tishlang
- **Homebrew**: `brew tap tishlang/tish https://github.com/tishlang/tish && brew install tish`

---

## If Homebrew Fails

- **“Release has no asset tish-darwin-arm64”** → The prerelease was promoted before the build workflow uploaded binaries. Promote only after the full CI run succeeds.
