# Publishing a release

## Prepare

- Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
- The private signing key lives in `%USERPROFILE%\.tauri\agentdeck.key`. Never commit it and keep it
  somewhere safe: without it, existing installs can no longer update.

## Build

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\agentdeck.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
pnpm tauri build
```

The output lands in `src-tauri/target/release/bundle/` (`nsis/*.exe`, `msi/*.msi` and their `.sig` files).

## latest.json

The app checks `https://github.com/0XLUC4/AgentDeck/releases/latest/download/latest.json`.

```json
{
  "version": "0.2.0",
  "notes": "What changed",
  "pub_date": "2026-11-01T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of the .exe.sig file>",
      "url": "https://github.com/0XLUC4/AgentDeck/releases/download/v0.2.0/AgentDeck_0.2.0_x64-setup.exe"
    }
  }
}
```

`signature` is the contents of the `.sig` file, not its path. `version` must be higher than the installed version.

## Publish

```powershell
gh release create v0.2.0 --title "AgentDeck 0.2.0" --notes "..." `
  src-tauri/target/release/bundle/nsis/AgentDeck_0.2.0_x64-setup.exe `
  src-tauri/target/release/bundle/msi/AgentDeck_0.2.0_x64_en-US.msi `
  latest.json
```
