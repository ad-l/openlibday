# openlibday

An open-source reimplementation of the engine for **Liberation Day** (1998),
the turn-based isometric tactics game. openlibday aims to play the original
game using data from a legally owned installation, on Windows, Linux and mobile.

openlibday contains **no original game assets or executable code**. You need an
existing Liberation Day installation; the engine reads its databases, artwork,
animation, video and sound files at runtime.

## Status

Planning. The first milestone is playable stand-alone skirmishes.
Campaign (story mode) and multiplayer will follow.

See [docs/PLAN.md](docs/PLAN.md) for the architecture and roadmap.
Implementation: Rust + SDL3. License: MIT (code only; no game data included).

## Knowledge base

Reverse-engineering notes, file formats and observed native behavior are in the
companion modding project (`libday/modding/README.md`, "Engine reconstruction
knowledge base"). Findings that the engine relies on are summarized in
`docs/` as they are implemented.
