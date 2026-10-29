# Ferrum Chess

An offline, native chess board written in Rust with iced, shakmaty and a local Stockfish process. There are no accounts, servers or online game services.

> **Status: experimental, not yet a verified release.** The Windows workflow and the engine/UI interaction have not been tested end-to-end. Do not treat the MSI or game behaviour as production-ready until CI and Windows smoke tests pass.

## Features and controls

- Play against Stockfish, or select **Analysis** to play both sides and display a best-move indicator and evaluation (from White's perspective). The selected search depth also controls difficulty.
- Click a piece and its destination, or press and drag to a square. Legal targets and the last move are highlighted. Choose the promotion piece **before** moving a pawn to the last rank.
- **New game** starts a standard game; choosing a side also starts a new game. **Restart** returns to the imported or standard starting position. **Undo** takes back a full pair of moves when playing the engine, or one ply in analysis.
- Open/save PGN using the file dialog. An imported FEN starting position is supported. Only the first game's main line is loaded; comments, variations and original metadata/results are not preserved. The exported result is derived from the current position, not from resignation or agreement.
- Engine searches and file dialogs run off the UI thread. A response from an old position is ignored after changing the position or settings.

## Build and run

Install a current stable Rust toolchain, including rustfmt and clippy. Linux also needs the native graphics/windowing libraries (for Debian/Ubuntu: `libxkbcommon-dev libwayland-dev libasound2-dev libudev-dev`). Then:

```sh
cargo build --release
cargo run --release
```

Install Stockfish separately for local builds. The program looks for `stockfish` (`stockfish.exe` on Windows) beside its own executable, then on `PATH`. Alternatively, set `FERRUM_STOCKFISH` to the absolute executable path. For example, after building place `stockfish.exe` next to `target/release/ferrum-chess.exe`. Errors starting the engine appear in the application. Stockfish itself is not checked into this repository.

## Windows installation and releases

The intended release workflow builds for Windows 10/11 x86-64. The MSI installs the application and Stockfish side by side and adds a Start menu shortcut. The portable ZIP contains both executables and notices; extract both into the **same directory**. The individually attached `ferrum-chess.exe` also needs the separately attached `stockfish.exe` in the same directory or on PATH. Installers are currently unsigned; Windows may display a SmartScreen warning.

To release, set the version in `Cargo.toml`, commit it, then push the matching `vX.Y.Z` tag. `.github/workflows/release.yml` runs cargo-dist and builds an MSI with WiX 4 separately because the MSI must include the engine. It checks installation and the engine's UCI readiness before publishing. A successful GitHub Actions run is required to establish that these artifacts actually exist; the workflow has not yet been exercised here. The workflow downloads Stockfish 17.1 from its official GitHub release and publishes its corresponding source archive.

## Screenshots

No verified screenshot is available yet. Run the application locally to view the board; screenshots should be captured from an actual successful build rather than a mockup.

## Known limitations

- Standard chess only; no Chess960 or other variants. Threefold repetition and 50/75-move draw claims are not tracked; checkmate, stalemate and insufficient material are detected by shakmaty. The FEN sent to the engine omits earlier repetition history.
- PGN support is intentionally main-line only. Annotated games lose comments, variations, tags and declared results on export; importing multiple games loads the first one only.
- The `stockfish` 0.2.11 wrapper uses blocking reads and can panic or hang if the executable is broken or returns unexpected UCI output; an obsolete search is ignored but not cancelled. Since release builds use `panic = "abort"`, such a panic can terminate the application. Use a trusted official Stockfish binary; robust timeouts/cancellation require a different or fixed adapter.
- The best-move hint is a line with an endpoint marker, not an arrowhead. Dragging selects on press and moves on release; the piece is not animated under the pointer. The board is fixed at 512 × 512 and is not yet responsive for small windows.
- No local Rust dependency build, clippy run, test run, GUI/engine integration, cargo-dist build, MSI installation, or GitHub Release has been verified in the current restricted environment. There is no committed `Cargo.lock` yet; dependency versions are therefore not reproducible. Validate and commit a lockfile before publishing a release.

## License and third-party software

Ferrum Chess's **own source code** is available under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. This does **not** mean the combined distributed binary is MIT/Apache-only: `shakmaty` and `pgn-reader` are GPL-3.0-or-later, and their inclusion imposes GPL obligations on the combined program. Stockfish is a separate GPL-3.0-or-later program; provide its license and corresponding source when distributing it. See [shakmaty GPL](assets/shakmaty-GPL.txt), [Stockfish source](https://github.com/official-stockfish/Stockfish/tree/sf_17.1) and the license notices in the distribution. Noto Sans Symbols 2 is licensed under the [SIL Open Font License 1.1](assets/OFL.txt). Consult counsel if needed before distributing binaries.
