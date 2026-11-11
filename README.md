# draw-mcp-connector
An MCP app for handwriting input and output

## Registering with your AI agent

Start Draw MCP Connector on your PC, then register it as an MCP server with the AI agent you use (first time only).

```sh
# Claude Code
claude mcp add --transport http draw http://127.0.0.1:47101/mcp

# Codex
codex mcp add draw --url http://127.0.0.1:47101/mcp
```

If your AI agent runs inside a Docker Desktop container, use `host.docker.internal` instead of `127.0.0.1` (inside a container, `127.0.0.1` points to the container itself).

```sh
claude mcp add --transport http draw http://host.docker.internal:47101/mcp
```

## Connecting the canvas

The canvas is a page you open in a browser. It works on an iPad, smartphone, tablet, PC, or any device whose browser supports WebGPU and WebTransport `serverCertificateHashes` (Safari 26.4 or later, Chrome, etc.).

1. Choose "Show QR Code" from the Draw MCP Connector icon in the menu bar (you can also ask your AI agent to "connect the canvas")
2. Scan the QR code with the camera of the device you will write on, and open the canvas
3. Press "Allow" in the dialog on your PC (the dot on the menu bar icon is orange while waiting for approval and green when connected)

Several devices can connect at once. Strokes are shared across all devices, and the menu shows one line per connected device with its state and IP address.

If the connection drops after you allowed it, reconnecting from the same tab within 1 minute (reloading the page, or the connection status at the top of the sidebar) connects again without the dialog. After closing the tab or after 1 minute, press "Allow" again. If another page takes over this page's connection and reconnects, this page is disconnected and a dialog tells you so.

## Opening in the browser on your PC

Choose "Open Canvas" from the Draw MCP Connector icon in the menu bar to open the canvas in this PC's default browser. As with the QR code, press "Allow" in the dialog on your PC to connect, and you can write with the mouse.

## Shapes and text drawn by the AI

With the MCP tool `draw_shapes`, AI agents can draw polylines, rectangles, ellipses, and text on the canvas. Sending shapes and strings instead of point lists keeps the AI's token usage low. What the AI draws becomes ordinary strokes on the canvas, just like handwriting.

## Building from source

The PC app (`src/bridge`) embeds the `BRIDGE_CANVAS_URL` environment variable at build time as the canvas page opened by the QR code and "Open Canvas". To use a canvas page you host yourself (HTTPS), specify its URL.

```sh
cd src/bridge
BRIDGE_CANVAS_URL=https://canvas.example.com/ cargo build --release
```

If it is not specified, the URL is `http://127.0.0.1:5173/`. This is the dev server for the canvas page running on the same PC, and it can only be opened from this PC's browser ("Open Canvas").

```sh
cd src/ui
npm ci
npm run dev
```

Builds on GitHub Actions (`.github/workflows/bridge.yml`) use the repository variable `BRIDGE_CANVAS_URL`.

## License

Draw MCP Connector (this app) is licensed under the MIT License (`LICENSE`). The bundled glyph data is under a different license, described below. This is also shown in "About Draw MCP Connector" in the menu.

The licenses of the crates built into the app are collected at build time by `src/bridge/tools/third_party_licenses.py` and placed in `Contents/Resources/THIRD-PARTY-LICENSES.txt` inside the .app. The licenses allowed for bundled crates are listed in `src/bridge/deny.toml`, and cargo-deny checks them in CI (along with advisories such as vulnerabilities). The licenses of the packages bundled into the canvas page are collected at build time by `src/ui/vite.config.ts` into `licenses.txt`, served with the page, and a comment at the top of the JavaScript points to it.

Text glyphs are converted from [KanjiVG](https://kanjivg.tagaini.net) (Copyright (C) 2009/2010/2011 Ulrich Apel, Creative Commons Attribution-Share Alike 3.0). The converted glyph data (`src/bridge/assets/kanjivg/glyphs.bin`) is distributed under the same license. See `src/bridge/assets/kanjivg/README.md` for details.
