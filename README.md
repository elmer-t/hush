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

Hush never goes online. Your thoughts stay as private as they deserve to be, which in my case is mostly a list of things I forgot to do.

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

Both commands first run `npm run fetch-model`, which downloads the bundled embedding model (about 34 MB) from Hugging Face into `src-tauri/models/` and checks it against the SHA-256s in `src-tauri/models/manifest.json`. A release build fails without it; `tauri dev` carries on and Hush shows "Indexing stopped" until the files are there. The build also downloads ONNX Runtime once (`ort`'s `download-binaries`). After that, Hush itself never goes online.

## Tests & checks

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
npm run check
```

The test that runs the real embedding model is skipped (it prints `SKIPPED`) unless `npm run fetch-model` has been run.

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
