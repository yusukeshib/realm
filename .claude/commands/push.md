Perform the release workflow:

1. Read the current version from `Cargo.toml` (line 3, format: `version = "X.Y.Z"`).
2. Bump the patch version (e.g. `0.0.5` -> `0.0.6`). Update the version in both:
   - `Cargo.toml` line 3
   - `flake.nix` line 16
3. Run `cargo fmt`.
4. Run `cargo clippy` — if there are any warnings, fix them.
5. Create a new branch named `feature/<feature-description>` (e.g. `feature/interactive-session-deletion`).
6. Review the diff (`git diff`) and commit with a descriptive message:
   - First line: a short summary of what changed (not just the version number)
   - Second line: blank
   - Third line: `vX.Y.Z` (the new version)
   For example: `Fix session serialization and centralize HOME resolution\n\nvX.Y.Z`
7. Push the branch to remote and create a pull request targeting `main`.
