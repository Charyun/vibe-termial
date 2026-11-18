# Agent icon sources

These images are bundled locally for offline use in the VibeTerm native desktop app. Agent names and marks belong to their respective owners.

Source repository: https://github.com/stablyai/orca

Pinned revision: `6df2fb11bc5da21c4d4888f6483201631c2a125d`

| Local image | Orca source | Adaptation |
| --- | --- | --- |
| `claude.svg` | `docs/assets/claude-logo.svg` | Unchanged SVG asset. |
| `codex.svg` | `src/renderer/src/components/status-bar/accounts/provider-icons.tsx`, `OpenAIIcon` | Exported the SVG as an image, retained the geometry, resolved `currentColor` to `#e0e6e0` for the dark interface. |
| `pi.svg` | `src/renderer/src/components/agent/icons/agent-icon-glyphs.tsx`, `PiIcon` | Exported the SVG as an image, retained the pi.dev favicon geometry used by Orca, resolved `currentColor` to `#e0e6e0`. |
| `gemini.png` | `src/shared/agent-icons/gemini.png` | Unchanged bundled PNG asset. |

Orca's MIT license is included in [LICENSE.orca](LICENSE.orca). The prototype uses the images through local `<img>` elements; no remote favicon requests are made at runtime.
