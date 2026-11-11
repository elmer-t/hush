## Native libraries, fonts and models

These are compiled into Hush, shipped next to it, or bundled as resources, but they are not Rust crates or npm packages in their own right, so the sections further down don't cover them. Versions are the ones in `Cargo.lock` and `package-lock.json` when this file was generated.

### SQLCipher (community edition)

SQLCipher {{sqlcipher}}, from the Rust crate libsqlite3-sys {{lock libsqlite3-sys}} (`bundled-sqlcipher-vendored-openssl`). Compiled from source and linked statically into Hush. Copyright Zetetic LLC. BSD 3-Clause licence:

{{text crate:libsqlite3-sys/sqlcipher/LICENSE}}

### SQLite

SQLite {{sqlite}}, inside the SQLCipher amalgamation above. SQLite is in the public domain. Its source carries this notice in place of a licence:

~~~~text
The author disclaims copyright to this source code.  In place of
a legal notice, here is a blessing:

   May you do good and not evil.
   May you find forgiveness for yourself and forgive others.
   May you share freely, never taking more than you give.
~~~~

### OpenSSL

OpenSSL {{openssl}}, from the Rust crate openssl-src {{lock openssl-src}}. Compiled from source and linked statically into Hush as SQLCipher's crypto provider. Copyright The OpenSSL Project Authors. Apache License 2.0:

{{text crate:openssl-src/openssl/LICENSE.txt}}

### Oniguruma

Oniguruma {{oniguruma}}, the regular expression library the tokenizer uses, from the Rust crate onig_sys {{lock onig_sys}}. Compiled from source and linked statically. BSD 2-Clause licence:

{{text crate:onig_sys/oniguruma/COPYING}}

### ONNX Runtime

ONNX Runtime 1.31.0 (Microsoft), which runs the embedding model. On Windows the Rust crate ort-sys {{lock ort-sys}} downloads a prebuilt static library (built by pyke.io with the DirectML provider) at build time and links it into Hush. MIT licence:

{{text vendored:onnxruntime-1.31.0-LICENSE.txt}}

ONNX Runtime includes third-party code under its own notices, reproduced in full:

{{text vendored:onnxruntime-1.31.0-ThirdPartyNotices.txt}}

### DirectML (Windows only)

DirectML 1.15.4 (Microsoft), the `DirectML.dll` next to `hush.exe` that lets ONNX Runtime use the GPU. It comes with the ONNX Runtime build above; 1.15.4 is the version ONNX Runtime 1.31.0 pins (`packages.config`). It is not open-source software. Microsoft Software License Terms, from the `Microsoft.AI.DirectML` 1.15.4 NuGet package:

{{text vendored:directml-1.15.4-LICENSE.txt}}

DirectML's own third-party notices:

{{text vendored:directml-1.15.4-ThirdPartyNotices.txt}}

### Microsoft Edge WebView2 loader (Windows only)

The WebView2 loader, from the Rust crate webview2-com-sys {{lock webview2-com-sys}}, linked statically into Hush on Windows. The window's web content runs in the Microsoft Edge WebView2 Runtime that is part of Windows; Hush does not ship the runtime. WebView2 SDK licence, from the `Microsoft.Web.WebView2` 1.0.3650.58 NuGet package:

{{text vendored:webview2-sdk-1.0.3650.58-LICENSE.txt}}

### Fonts

IBM Plex Sans {{npm @fontsource/ibm-plex-sans}}, IBM Plex Mono {{npm @fontsource/ibm-plex-mono}} (both copyright IBM Corp.) and Source Serif 4 {{npm @fontsource-variable/source-serif-4}} (Adobe), bundled through the Fontsource npm packages. All three are under the SIL Open Font License 1.1; the full licence text for each is in the npm section below, under the `@fontsource` packages.

### Embedding models

- **bge-small-en-v1.5** (BAAI), the int8 ONNX export by Xenova. Shipped with Hush as a resource. MIT licence:

{{text repo:src-tauri/models/bge-small-en-v1.5.LICENSE.txt}}

- **multilingual-e5-small** (intfloat, MIT) and **all-MiniLM-L6-v2** (sentence-transformers, Apache License 2.0), and the fp16 export of bge-small-en-v1.5. Not shipped with Hush: Settings → Model downloads them from Hugging Face when the user asks. Each model's licence is linked there.
