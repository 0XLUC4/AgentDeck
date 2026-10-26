# Publier une version d'AgentDeck

## 1. Préparer

- Monter la version dans `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` et `package.json` (mêmes numéros).
- La clé privée de signature des mises à jour est dans `%USERPROFILE%\.tauri\agentdeck.key` (sans mot de passe).
  **Ne jamais la committer. La sauvegarder** (gestionnaire de mots de passe) : sans elle, les installations
  existantes ne pourront plus accepter de mise à jour. La clé publique est dans `plugins.updater.pubkey`.

## 2. Construire (Windows, PowerShell)

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\agentdeck.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
pnpm tauri build
```

Sortie dans `src-tauri/target/release/bundle/` :

- `nsis/AgentDeck_<version>_x64-setup.exe` + `.sig` (installeur par utilisateur, FR/EN)
- `msi/AgentDeck_<version>_x64_fr-FR.msi` / `_en-US.msi` + `.sig`

## 3. `latest.json`

L'application interroge `https://agentdeck-updates.pages.dev/latest.json` (voir `plugins.updater.endpoints`).

```json
{
  "version": "0.2.0",
  "notes": "Nouveautés de la version",
  "pub_date": "2026-11-01T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contenu du fichier AgentDeck_0.2.0_x64-setup.exe.sig>",
      "url": "https://agentdeck-updates.pages.dev/AgentDeck_0.2.0_x64-setup.exe"
    }
  }
}
```

- `signature` = le **contenu** du `.sig`, pas son chemin.
- `version` doit être supérieure à la version installée (semver), sinon rien n'est proposé.

## 4. Héberger sur Cloudflare (plus tard)

**Pages** (le plus simple, fichiers ≤ 25 Mo) :

```powershell
mkdir release; copy src-tauri\target\release\bundle\nsis\*.exe release\; # + latest.json
pnpm dlx wrangler pages deploy release --project-name agentdeck-updates
```

**R2** (gros fichiers) : déposer l'installeur dans un bucket public (domaine personnalisé ou `r2.dev`),
mettre son URL dans `latest.json`, et servir `latest.json` depuis Pages ou R2.
Si l'endpoint change, mettre à jour `plugins.updater.endpoints` **avant** de distribuer la version :
les versions déjà installées ne connaissent que l'ancienne URL.
