<p align="center"><img src="docs/brand/neox-logo.svg" width="128" alt="NEOX logo"></p>

# NEOX: Windows optimizer (Rust + Tauri)

NEOX tunes Windows 10/11 for gaming. Hundreds of documented tweaks with their benefits and side effects, profiles, undo for every change, network tools, a Keyboard and a Mouse page, Auto-Tune, monitoring and cleanup.

## Download
Builds are published by GitHub Actions from the `recode` branch: open the [latest release](https://github.com/KarolDuda/NEOX/releases/latest) and download `Neox_Setup.exe`. Installed copies update themselves: Settings, Check for updates, Install. The installer is not code-signed, so SmartScreen may warn you (More info, then Run anyway). Each release lists the SHA-256 of the files and a build attestation you can verify with `gh attestation verify Neox_Setup.exe --repo KarolDuda/NEOX`.

## Free and Premium
- **Free**: a set of safe optimizations, the basics on the Keyboard and Mouse pages, network repair, the DNS switcher, undo, history, restore point and the disk analyzer.
- **Premium**: every optimization (including the high-risk performance items), the full Keyboard and Mouse pages, Auto-Tune, Internet Auto-Tune, profiles, Game Boost, Memory Cleaner, Performance Score and game tools. A 7-day trial is available. Licenses are bound to one PC.

## Safety
- Every change takes a snapshot first, is read back afterwards and is rolled back if Windows did not accept it. Everything can be reverted from History.
- Risky items (experimental and high risk) show a warning and need a confirmation. NEOX never touches Defender, Windows Update, Firewall or other critical services.
- Network items that can slow the Internet (global offload off, connectivity probe off, DNS negative cache off and similar) are never part of a profile or Auto-Tune. "Repair my internet" on the Network page undoes them and restores the Windows TCP defaults.
- Back up your system before using the extreme items. NEOX shows this reminder on every launch.

## Layout
```
crates/neox-core   catalog (data/tweaks.json), engine, profiles, Auto-Tune, license (no Windows APIs)
crates/neox-sys    registry, powercfg, services, bcdedit, network, cleanup (Windows layer)
app/               Tauri shell and commands
ui/                the interface (plain HTML, CSS and JavaScript, no build step)
installer/         Inno Setup
docs/              architecture, testing, brand assets
```

Author and publisher: spinprint (spinprint.pl).
