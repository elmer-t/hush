# Hush

Private notes, journals and thoughts in a single encrypted SQLite (SQLCipher) vault. Desktop app built with Tauri 2, SvelteKit and TypeScript.

See [docs/project-brief.md](docs/project-brief.md) and [docs/decisions.md](docs/decisions.md).

## Prerequisites (Windows)

```powershell
winget install Rustlang.Rustup
winget install StrawberryPerl.StrawberryPerl
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable-msvc
```

Plus Node.js (LTS or newer). WebView2 ships with Windows 11.

- **Perl** is needed because OpenSSL (SQLCipher's crypto provider) is compiled from source and linked statically. Strawberry Perl must come before Git for Windows' `perl.exe` on `PATH`; check with `where.exe perl`.
- **MSVC:** Rust uses the newest Visual Studio install it finds. If that install has an incomplete C++ toolset, linking fails with `LNK1104: cannot open file 'msvcrt.lib'`. Either add the "MSVC x64/x86 build tools" component to that install, or build from a *Developer PowerShell for VS 2022*.

## Build & run

```powershell
npm install
npm run tauri dev      # run in development
npm run tauri build    # release build + installer
```

The first build compiles OpenSSL and takes several minutes.

Both commands first run `npm run fetch-model`, which downloads the bundled embedding model (about 34 MB) from Hugging Face into `src-tauri/models/` and checks it against the SHA-256s in `src-tauri/models/manifest.json`. A release build fails without it; `tauri dev` carries on and Hush shows "Indexing stopped" until the files are there. The build also downloads ONNX Runtime once (`ort`'s `download-binaries`). Hush itself never goes online.

## Tests & checks

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
npm run check
```

The test that runs the real embedding model is skipped (it prints `SKIPPED`) unless `npm run fetch-model` has been run.
