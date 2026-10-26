# Plan — Dashboard multi-agents (Tauri)

> Nom de travail : **AgentDeck** (à changer)
> Objectif : une app de bureau où on ouvre plusieurs terminaux côte à côte, chacun avec un agent IA (Claude Code, Codex, Gemini CLI), avec une barre d'état par agent et un widget vocal flottant pour leur parler directement.

---

## 1. Stack technique

| Couche | Choix | Pourquoi |
|---|---|---|
| App de bureau | **Tauri v2** | Léger, rapide, multi-fenêtres natif |
| Cœur (backend) | **Rust** | Gestion des terminaux, audio, raccourcis globaux |
| Interface | **React + TypeScript + Vite** | Tu connais déjà le TSX |
| Rendu terminal | **xterm.js** (+ addons `fit`, `webgl`, `search`) | Standard (VS Code l'utilise) |
| Terminaux réels | crate **`portable-pty`** | Marche sur Windows (ConPTY), macOS, Linux |
| Mise en page | **`dockview`** ou **`react-resizable-panels`** | Panneaux redimensionnables / déplaçables |
| État UI | **Zustand** | Simple, peu de code |
| Style | **Tailwind CSS** | Rapide pour un thème sombre propre |
| Raccourci global | **`tauri-plugin-global-shortcut`** | Touche qui marche même si l'app n'a pas le focus |
| Audio | crate **`cpal`** (capture micro) | Côté Rust, pas de souci de permission webview |
| Transcription | **`whisper-rs`** (whisper.cpp local) ou API (Whisper / Deepgram) | Local = gratuit et privé ; API = plus rapide sur petit PC |
| Persistance | **`tauri-plugin-store`** | Sauver la disposition et les sessions |

---

## 2. Fonctionnalités

### 2.1 Grille de terminaux
- Écran principal = grille de panneaux, chacun = un terminal avec un agent.
- Bouton **« + Ajouter un agent »** → fenêtre de choix :
  - Choix de l'agent : **Claude Code**, **Codex**, **Gemini CLI** (cartes avec logo, nom, statut « installé / non trouvé »).
  - Choix du dossier de travail.
  - Option : créer un **worktree Git** dédié (pour que les agents ne se marchent pas dessus).
  - Arguments optionnels (modèle, mode de permissions…).
- Panneaux redimensionnables, déplaçables, agrandir un panneau en plein écran (double-clic sur l'en-tête), fermer.
- Possibilité de **détacher un panneau dans une fenêtre séparée** (Tauri multi-fenêtres) — utile sur deux écrans.
- Onglets en haut pour avoir plusieurs « sessions » (ex. un projet par onglet).

### 2.2 Logos des agents
- Mettre les logos dans `src/assets/agents/` (SVG).
- Utiliser les logos officiels fournis par chaque marque dans leurs kits presse, en respectant leurs règles d'usage (pas de modification, taille minimale, etc.).
- Chaque agent a aussi une **couleur d'accent** (bordure du panneau, pastille) pour les reconnaître d'un coup d'œil.

### 2.3 Barre d'infos au-dessus de chaque terminal
Contenu de l'en-tête du panneau :
- Logo + nom de l'agent + titre de la tâche (éditable)
- **Statut** avec pastille de couleur :
  - 🟢 terminé · 🔵 en train de travailler · 🟠 attend ta réponse / une permission · 🔴 erreur · ⚪ inactif
- Dernière action (ex. « Édite `src/STYLE.md` », « Lance une commande shell »)
- Temps écoulé sur la tâche en cours
- Compteur de tâches terminées
- Boutons : relancer, agrandir, détacher, fermer

**Comment savoir ce que fait l'agent (du plus fiable au moins fiable) :**
1. **Hooks officiels** — l'app lance un petit serveur HTTP local (ex. `localhost:47800`) et configure les hooks de l'agent pour y envoyer des événements :
   - Claude Code : hooks `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop` (type `http` ou commande `curl`).
   - Codex : option `notify` / hooks dans `~/.codex/config.toml`.
   - Gemini CLI : vérifier le système de hooks disponible au moment du dev.
   - Chaque panneau a un identifiant envoyé via une variable d'environnement (`AGENTDECK_PANE_ID`) pour savoir quel panneau a émis l'événement.
2. **Analyse de la sortie** du terminal (repérer « Worked for… », prompts de permission, etc.) — en secours.
3. **Détection d'inactivité** : pas de sortie depuis X secondes → « inactif / attend ».

**Notifications** : notification système + son quand un agent passe à « terminé » ou « attend ta réponse » (`tauri-plugin-notification`).

### 2.4 Widget vocal (pilule flottante)
- Une **fenêtre Tauri séparée** : transparente, sans bordure, **toujours au premier plan**, centrée en bas de l'écran (déplaçable).
- Forme : **pilule noire arrondie** avec des barres qui s'animent selon le volume du micro (comme ta capture).
- Visible **par-dessus toutes les fenêtres**, même si l'app est en arrière-plan.

**Fonctionnement :**
1. Raccourci global (ex. `Ctrl+Espace` ou une touche configurable).
2. Mode au choix : **maintenir pour parler** (push-to-talk) ou **appuyer / ré-appuyer**.
3. Pendant l'écoute : la pilule apparaît, les barres bougent.
4. Au relâchement : transcription → le texte est **tapé dans le terminal de l'agent actif** (celui sélectionné / avec le focus).
5. Option : aperçu du texte dans la pilule avant envoi (Entrée pour valider, Échap pour annuler).
6. Option bonus : commencer la phrase par le nom de l'agent pour le cibler (« Codex, lance les tests »).

**Détails techniques :**
- Capture audio avec `cpal` côté Rust, rééchantillonnage en 16 kHz mono pour Whisper.
- Le niveau sonore (RMS) est envoyé à la pilule ~30 fois/s via un événement Tauri pour animer les barres.
- Écriture dans le PTY du panneau actif + `\r` pour valider (configurable : envoyer ou juste écrire).

---

## 3. Architecture

```
agent-deck/
├── src-tauri/                 # Backend Rust
│   ├── src/
│   │   ├── main.rs
│   │   ├── pty/               # Création/gestion des terminaux (portable-pty)
│   │   ├── agents/            # Définition Claude / Codex / Gemini, détection d'installation
│   │   ├── hooks_server/      # Serveur HTTP local qui reçoit les événements des agents
│   │   ├── voice/             # Capture micro (cpal) + transcription (whisper-rs ou API)
│   │   ├── shortcuts.rs       # Raccourcis globaux
│   │   └── windows.rs         # Fenêtre principale, fenêtres détachées, pilule vocale
│   └── tauri.conf.json
├── src/                       # Interface React
│   ├── app/                   # Layout principal, onglets de session
│   ├── components/
│   │   ├── TerminalPane/      # xterm.js + en-tête d'infos
│   │   ├── AgentPicker/       # Choix de l'agent avec logos
│   │   ├── StatusBadge/
│   │   └── VoicePill/         # La pilule animée
│   ├── store/                 # Zustand : panneaux, statuts, réglages
│   ├── assets/agents/         # Logos SVG
│   └── voice.html             # Point d'entrée de la fenêtre pilule
└── package.json
```

**Communication front ↔ Rust (commandes et événements Tauri) :**
- `spawn_agent(agent, cwd, args) -> pane_id`
- `write_to_pane(pane_id, data)` / `resize_pane(pane_id, cols, rows)` / `kill_pane(pane_id)`
- Événement `pane-output:{id}` → données du terminal vers xterm.js
- Événement `agent-status` → mise à jour de l'en-tête
- Événements `voice-level`, `voice-transcript`, `voice-state`

---

## 4. Étapes de développement

### Étape 1 — Base (1-2 jours)
- [ ] Créer le projet : `npm create tauri-app@latest` (React + TS)
- [ ] Ajouter Tailwind, Zustand, xterm.js
- [ ] Un seul terminal qui lance un shell via `portable-pty` et s'affiche dans xterm.js
- [ ] Gérer le redimensionnement (addon `fit` + `resize_pane`)

### Étape 2 — Multi-terminaux + choix de l'agent (2-3 jours)
- [ ] Grille de panneaux redimensionnables (dockview)
- [ ] Fenêtre « Ajouter un agent » avec les 3 cartes et logos
- [ ] Détection : `claude`, `codex`, `gemini` sont-ils installés ? (lien vers l'installation sinon)
- [ ] Choix du dossier + option worktree Git
- [ ] Sauvegarde/restauration de la disposition

### Étape 3 — Barre d'infos et statuts (2-3 jours)
- [ ] Serveur HTTP local pour les hooks
- [ ] Configuration automatique des hooks Claude Code (dans les réglages du projet, sans écraser ceux existants)
- [ ] Idem Codex, puis Gemini
- [ ] En-tête avec pastille, dernière action, chrono, compteur
- [ ] Notifications système + son

### Étape 4 — Widget vocal (3-4 jours)
- [ ] Fenêtre pilule transparente, toujours au premier plan
- [ ] Raccourci global push-to-talk
- [ ] Capture micro + animation des barres
- [ ] Transcription (commencer par une API pour aller vite, puis whisper local)
- [ ] Envoi du texte au panneau actif

### Étape 5 — Finitions
- [ ] Panneaux détachables en fenêtres séparées
- [ ] Thèmes (sombre par défaut)
- [ ] Page de réglages (raccourcis, modèle Whisper, mode d'envoi)
- [ ] Installeurs Windows / macOS (`tauri build`)

---

## 5. Points d'attention

- **Windows** : `portable-pty` utilise ConPTY, il faut Windows 10 1809+. Bien tester les couleurs et le redimensionnement.
- **macOS** : déclarer `NSMicrophoneUsageDescription` dans `Info.plist`, sinon le micro est bloqué.
- **Linux** : les fenêtres transparentes dépendent du gestionnaire de fenêtres.
- **Performance** : avec beaucoup de panneaux, regrouper la sortie des PTY (envoyer toutes les ~16 ms plutôt qu'à chaque octet) et activer l'addon WebGL de xterm.js.
- **Hooks** : ne jamais écraser la config existante de l'utilisateur, fusionner proprement et pouvoir désinstaller.
- **Whisper local** : le modèle `base` ou `small` suffit pour des commandes ; `large` est lent sans GPU.
- **Sécurité** : le serveur de hooks n'écoute que sur `127.0.0.1` avec un jeton secret.

---

## 6. Idées pour plus tard
- Un agent « chef d'orchestre » qui répartit les tâches entre les autres
- Historique des sessions avec recherche
- Coût / quota utilisé par agent
- Synthèse vocale : l'agent te lit un résumé quand il a fini
- Mode « mur » : vue compacte de 6-9 agents avec juste les statuts
