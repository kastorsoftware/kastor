# Kastor

[<kbd>🇷🇺 Русский</kbd>](README.ru.md)

Desktop application for working with Telegram accounts. The UI is built with React; the native layer uses Rust and Tauri.

> This project is released under [The Unlicense](LICENSE). You may use, modify, publish, and sell the code without conditions or attribution. It is provided as is, without warranties.

## Features

- import, authorize, and manage Telegram accounts;
- work with proxies, validate them, and distribute them among accounts;
- check accounts, convert sessions, and perform batch actions;
- tools for messaging, parsing, inviting, auto-replies, cloning, and other workflows;
- local SQLite data storage and a background task queue.

Use the application lawfully and in compliance with Telegram's rules. You are responsible for the accounts you use.

## Development

Node.js, Rust, and the Windows Tauri system dependencies are required. The Rust MSVC toolchain is strongly recommended for builds.

```bash
npm install
npm run tauri dev
```

Build the frontend:

```bash
npm run build
```

Build the desktop application:

```bash
npm run tauri build
```

## Project structure

- `src/` — React frontend;
- `src-tauri/` — Rust/Tauri backend and MTProto client.

## Roadmap

Improving existing functionality is always the first priority.

- [x] Update the Telegram API to Layer 228.
- [ ] Expand the Stories module.
- [ ] Add macOS support.
- [ ] Add new modules when they are needed.

## Known issues

Linux and macOS are not supported yet.

## Reporting issues and contributing

If you find a bug, the worst thing to do is stay silent or send an angry message. The most helpful thing is to open an issue with a clear description: what you did, what you expected, what actually happened, and any relevant logs or screenshots. Well-described reports are much more likely to be resolved soon.

Feature ideas are welcome as issues. Pull requests are welcome too — please clearly explain the problem they solve and keep them focused.

## Code status

About 85% of this project was created with AI; it is a hobby project made for fun. No guarantees are made that it works correctly or that using it will be safe for your accounts. Review the code before using it in sensitive or production scenarios.
