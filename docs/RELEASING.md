# Releasing ttask

Releases are built by [`.github/workflows/release.yml`](../.github/workflows/release.yml)
on a Windows runner with the same `scripts/build-release.ps1` used locally.

## Cutting a release

1. Bump `version` in the workspace `Cargo.toml`, run `cargo check` (updates
   `Cargo.lock`), commit.
2. Tag and push:

   ```sh
   git tag v0.3.1
   git push origin main v0.3.1
   ```

The workflow then:

- checks the tag matches the workspace version, builds the static exe, runs
  `ttask --version` as a smoke test;
- generates `THIRD-PARTY-LICENSES.txt` with `cargo about` (`about.toml`,
  `about.hbs`); a dependency under a license not listed in `about.toml` fails
  the build until it is reviewed and added;
- attests the exe's build provenance and publishes the GitHub release with
  the exe, its `.sha256` and the license file, release notes generated from
  the commits;
- rewrites `bucket/ttask.json` (this repo doubles as a Scoop bucket) and
  commits it to `main`;
- opens a WinGet pull request, once that is set up (below).

To try the build without releasing: Actions → Release → Run workflow; the exe
is attached to the run as an artifact.

A broken release: `gh release delete vX.Y.Z --cleanup-tag`, fix, tag again.
Scoop users already on it get the next version with `scoop update`.

## WinGet (one-time setup)

WinGet installs the exe as a *portable* package: it lands in
`%LOCALAPPDATA%\Microsoft\WinGet\Packages\` with a `ttask` link on the PATH,
and `winget upgrade --all` updates it.

1. After the first GitHub release, submit the package by hand once:

   ```powershell
   winget install Microsoft.WingetCreate
   wingetcreate new https://github.com/stobjasz/ttask/releases/download/v0.3.1/ttask-0.3.1-x86_64-pc-windows-msvc.exe
   ```

   Use the identifier `stobjasz.ttask` (the workflow expects it), installer
   type `portable`, command alias `ttask`. Microsoft reviews the pull request,
   usually within a few days.
2. Fork [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)
   to your account.
3. Create a *classic* personal access token with the `public_repo` and
   `workflow` scopes and save it as the repository secret `WINGET_TOKEN`
   (Settings → Secrets and variables → Actions).

From then on every release opens the WinGet pull request automatically; the
job is skipped while the secret is missing.

## Code signing

The exe is not signed, so a browser download shows a SmartScreen warning
until the file earns reputation. Options for an open-source project run by an
individual outside the US/Canada: [SignPath Foundation](https://signpath.org)
(free for OSS, signs in CI) or Certum's Open Source Code Signing certificate.
