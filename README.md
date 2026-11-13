# Hush

A desktop app for private notes, journals and thoughts, kept in a single encrypted SQLite (SQLCipher) vault. Built with Tauri 2, SvelteKit and TypeScript, mostly by an AI, for an audience of exactly one.

Nobody asked for this. Obsidian exists. Plain text files in an encrypted folder exist. Paper exists and has excellent battery life. I wanted my journal in one encrypted file that I can search by meaning, and I wanted to see how far I could get by writing GitHub issues and letting Claude do the typing. This repository is the result of both.

If you found this by accident: welcome. You are now the second user. There is no roadmap, no support and no Discord. There are tests, though.

## What it does

- Opens a vault with a passphrase. The passphrase and key never touch the disk.
- Markdown notes with `[[wikilinks]]` and backlinks.
- Tags, topics, authors and dates.
- Full-text search (FTS5), plus semantic search with an embedding model that runs locally.
- Imports a folder of Markdown files (an Obsidian vault, say) and cleans up the HTML and 80-character line breaks on the way in.
- Locks itself when you walk away.

Hush goes online only when you click *Download* in Settings → Model, and then only to that model's pinned URL; the files are checked against a pinned SHA-256 before use. Otherwise your thoughts stay as private as they deserve to be, which in my case is mostly a list of things I forgot to do.

For the serious version, see [docs/project-brief.md](docs/project-brief.md) and [docs/decisions.md](docs/decisions.md).

## Prerequisites (Windows)

```powershell
winget install Rustlang.Rustup
winget install StrawberryPerl.StrawberryPerl
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable-msvc
```

Plus Node.js (LTS or newer). WebView2 ships with Windows 11.

Yes, that is a lot of toolchain for a notes app.

- Perl is needed because OpenSSL (SQLCipher's crypto provider) is compiled from source and linked statically. Strawberry Perl must come before Git for Windows' `perl.exe` on `PATH`; check with `where.exe perl`.
- Rust uses the newest Visual Studio install it finds. If that install has an incomplete C++ toolset, linking fails with `LNK1104: cannot open file 'msvcrt.lib'`. Either add the "MSVC x64/x86 build tools" component to that install, or build from a *Developer PowerShell for VS 2022*.

## Build & run

```powershell
npm install
npm run tauri dev      # run in development
npm run tauri build    # release build + installer
```

The first build compiles OpenSSL and takes several minutes. Make coffee. Write a journal entry about waiting for OpenSSL, if you already have a working build.

Both commands first run `npm run fetch-model`, which downloads the bundled embedding model (about 34 MB) from Hugging Face into `src-tauri/models/` and checks it against the SHA-256s in `src-tauri/models/manifest.json`. A release build fails without it; `tauri dev` carries on and Hush shows "Indexing stopped" until the files are there. The build also downloads ONNX Runtime once (`ort`'s `download-binaries`). After that, the app itself goes online only when you click *Download* in Settings → Model, to a pinned URL, and checks what it downloads by SHA-256.

## Releasing

The installer is built by GitHub Actions ([release.yml](.github/workflows/release.yml)), so cutting a release needs no local toolchain.

1. Set the same version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, run `cargo check --manifest-path src-tauri/Cargo.toml` so `Cargo.lock` follows, and merge that.
2. Tag that commit and push the tag:

   ```powershell
   git tag v0.3.0
   git push origin v0.3.0
   ```

3. The workflow builds `Hush_<version>_x64-setup.exe`, and attaches it with a `SHA256SUMS.txt` to a **draft** release. The build fails when the tag and the three versions disagree.
4. Install the draft's `setup.exe` on a real machine, open a vault, then publish the draft on GitHub.

To get an installer without releasing, run the workflow by hand (Actions → Release → Run workflow) and download the `hush-windows-x64` artifact. It also runs on pull requests that change the workflow or the bundle config.

The installer is per-user (no administrator rights) and is not code-signed, so Windows SmartScreen warns on first run. There is no updater: a new version is a new download.

## Tests & checks

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
npm run check
npm test
```

`npm test` leaves out the editor's frame-budget checks (typing into a 5,000-line note within 16 ms a keystroke), because on a busy machine they fail for no reason. Run them on an idle machine with `npm run test:perf`.

The test that runs the real embedding model is skipped (it prints `SKIPPED`) unless `npm run fetch-model` has been run.

## Security

The vault protects the file at rest: it is a SQLCipher database, with the key derived from your passphrase by PBKDF2-HMAC-SHA512. It does not protect a running app that is unlocked, a machine with malware on it, or a weak passphrase.

Found a vulnerability? Please report it privately as described in [SECURITY.md](SECURITY.md), not in a public issue.

## Third-party notices

[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) holds the licences of everything Hush ships: Rust crates, the npm packages in the bundle, and the native libraries, fonts and model. It goes into the installer and shows under Settings → About. After changing dependencies, regenerate it:

```powershell
cargo install --locked --features cli cargo-about   # once
npm run notices            # rewrites THIRD_PARTY_NOTICES.md (and build/)
npm run notices -- --check # fails when the committed file is out of date
```

The hand-written part is [scripts/notices/manual.md](scripts/notices/manual.md). The script lists every licence family it finds and flags copyleft or unknown licences.

## How this gets built

I write issues and label them `ready-for-agent`. An agent picks the lowest-numbered one, opens a branch and a PR, and lists the calls it made under a `Decisions` heading so I can overrule them in review. It stops and asks only before something that could make an existing vault unreadable. The rules are in [CLAUDE.md](CLAUDE.md).

## Contributing

You could. I would be surprised.

## Licence

MIT. See [LICENSE](LICENSE).
