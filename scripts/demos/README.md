# Neovim demo recordings

These are real terminal recordings made by VHS, using the installed Neovim configuration. No completion results, diagnostics, edits, or build messages are simulated. `session.lua` positions the cursor, prepares split views, and samples evidence; the tape sends the editing keys.

The checked-in media was recorded with the maintainer's `~/.config/nvim`, Neovim 0.12.6, Catppuccin, Blink, Noice, code-actions.nvim, and the configuration's TeXquill linked editing adapter. A different configuration can change menus, bindings, completion acceptance, and linked editing behavior.

## Record

Install `nvim`, `vhs` (and its browser/ttyd dependencies), `ffmpeg`, and ImageMagick (`magick`). The build scene also needs `tex-fmt`, `latexmk`, and a TeX distribution. The current presentation uses the installed **Cascadia Code NF** font and macOS's Helvetica font for the caption.

From the repository root:

```sh
cargo build --release --locked
python3 scripts/demos/record.py
```

To regenerate selected clips or choose a different binary/output directory:

```sh
python3 scripts/demos/record.py completion navigation
python3 scripts/demos/record.py linked-editing --server "$PWD/target/release/texquill" --output /private/tmp/texquill-clips
```

VHS starts a local terminal service and a browser. It needs permission to bind a loopback port when running in a restricted sandbox.

Each scene gets a separate temporary copy of `fixtures/`. The runner sets `NVIM_TEXQUILL` to the chosen server binary; this environment variable is supported by the maintainer's configuration. Other configurations must likewise launch that binary and support the recorded bindings. `XDG_CONFIG_HOME` and `XDG_DATA_HOME` are inherited so the installed configuration and plugins are used.

## Recording adjustments

Changes apply only to the disposable Neovim processes:

- `NVIM_SMOKE_TEST=1` prevents plugin installation and automatic session saving in the maintainer's configuration. `-i NONE -n` disables ShaDa and swap files; cache/state files go to the temporary directory.
- Copilot suggestions are disabled for the disposable session to keep the recording focused on TeXquill. The configuration files are not edited.
- Conceal and relative line numbers are disabled in the opening window for readable source. The theme, statusline, completion, popups, and editing integrations come from the installed configuration.
- Temporary **Ctrl-j** bindings prepare a second view, move to the signature example, or call `vim.lsp.buf.format({ name = 'texquill' })`. They do not provide language results.
- Project settings disable builds on save in the editing scenes. The build scene keeps the user's build-on-save setting enabled: the manual build mapping saves the formatted source, which starts compilation.
- A static title strip is added above the terminal recording. Startup and shutdown are omitted; the visible typing and waits retain their recorded timing. There is no audio.

## Outputs and verification

`docs/public/demos/` contains H.264 MP4 (1280 × 780), looping GIF (960 × 585), and PNG posters. The MP4 files have fast-start metadata for browser playback. The website uses posters and `preload="none"`, with user-controlled playback.

Raw recordings and sampled UI evidence are kept under the ignored `target/demos/`. Evidence includes local paths and must stay out of public assets. Before export, the script checks:

- The active client is `texquill`; citation completion came from LSP and inserted the expected citation key.
- Definition navigation reached the included source, and rename updated both files.
- The undefined-reference warning appeared, its suggested edit applied, and the warning cleared.
- Both environment names changed together and a single undo restored them.
- Hover and signature popups contain the real cross-file macro documentation.
- Formatting indented the source, and compilation produced `main.pdf`.

Run `npm run docs:build` after regenerating or changing the documentation. Inspect representative frames from each video as well as the generated pages before publishing.

## Native forward and reverse search recordings

`record-synctex.py` captures separate **live application windows** with macOS `screencapture`, then combines their simultaneous footage. It requires Screen Recording permission for the invoking process, kitty, Skim, `synctex`, and the existing tex-tools.nvim viewer integration. The user's Skim editor callback and the demo Neovim must share the same inverse-search registry; the script preserves `XDG_CACHE_HOME` for this reason. It does not change Skim preferences.

```sh
python3 scripts/demos/record-synctex.py prepare
```

This opens a dedicated kitty/Neovim process using the installed configuration, builds the three-page project from `synctex/` with TeXquill, and opens its PDF through tex-tools. Only the disposable project under `target/demos/synctex-project/` is used. The helper preloads the two source buffers so the existing viewer callback can match this session.

Identify the dedicated kitty window and this PDF window in the macOS window list; pass their actual CGWindow IDs, not IDs from a previous run. Put Skim on **page 1** for forward search, then run:

```sh
python3 scripts/demos/record-synctex.py capture forward-search --source-window KITTY_WINDOW_ID --pdf-window SKIM_WINDOW_ID
```

After three seconds the script calls the same tex-tools API as the user's forward-search mapping. The video must show page 1 changing to page 3; the script also checks the actual `latex.forwardSearch` response.

For reverse search, first put Skim on **page 2**, then run:

```sh
python3 scripts/demos/record-synctex.py capture reverse-search --source-window KITTY_WINDOW_ID --pdf-window SKIM_WINDOW_ID
```

During the 12-second recording, **Shift-Command-click** the paragraph beginning “The method compares predicted values” in Skim. The callback must open `sections/method.tex` at line 4; the recorder checks the real `TexToolsInverseSearch` event and current editor buffer. This is Skim's native mapping plus the configured callback, rather than the server's separate coordinate-based `latex.inverseSearch` request.

Both clips export MP4/GIF/posters in the same sizes as the terminal clips. Their title strip is added during export; the lower blank part of the PDF is cropped. Raw window footage and verification evidence remain under `target/demos/`. After recording, close only the dedicated demo Neovim and its PDF window.
