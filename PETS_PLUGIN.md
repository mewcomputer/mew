
## code buddy game plan

buddy is a persistent digital companion integrated into a agent harness CLI/TUI, or external device. it generates a unique pet per user, displays it as an ASCII sprite beside the input prompt, and occasionally reacts to conversations with short quips.

### what you're building

a modular companion system with six components:

1. **type layer** — shared data structures
2. **generation engine** — deterministic RNG-based companion creation
3. **sprite library** — ASCII art frames + hat overlays
4. **react/ink component** — animated terminal UI with speech bubble
5. **system prompt integration** — introduces companion to the LLM
6. **command handler** — slash command with subcommands

---

### component 1: types

define these shapes first, everything else references them.

```
Rarity: common | uncommon | rare | epic | legendary
  - weights: 60 / 25 / 10 / 4 / 1
  - star display: ★ through ★★★★★

Species: 18 types (duck, goose, blob, cat, dragon, octopus, owl, penguin,
  turtle, snail, ghost, axolotl, capybara, cactus, robot, rabbit, mushroom, chonk)

Eyes: [ ·, ✦, ×, ◉, @, ° ]
Hats: [ none, crown, tophat, propeller, halo, wizard, beanie, tinyduck ]

Stats: DEBUGGING | PATIENCE | CHAOS | WISDOM | SNARK
  - values 1-100
  - one "peak" stat, one "dump" stat, rest scattered
  - rarity sets the floor (common=5, legendary=50)

CompanionBones: { species, eyes, hat, shiny, rarity, stats }
  — derived from userId hash, never persisted

CompanionSoul: { name, personality }
  — user-facing identity, persisted in config

StoredCompanion: CompanionSoul & { hatchedAt: Date }
  — the only thing you write to disk

Companion: CompanionBones & CompanionSoul & { hatchedAt }
  — full runtime object, assembled on read
```

key design: bones are never stored. regenerate them from the userId hash on every read. this means the species list can evolve without breaking old companions, and users can't manually edit their way to legendary.

---

### component 2: generation engine

**deterministic RNG from userId:**
- hash: FNV-1a or any fast non-cryptographic hash. hash `userId + 'some-salt-string'` to get a seed
- PRNG: Mulberry32 (tiny, pure, seeded). use this for all rolls
- cache the result for a given userId — bones recompute on every render tick, which is expensive

**generation order:**
1. roll rarity (weighted)
2. roll species, eyes, hat uniformly from their lists
3. roll shiny (1% flat)
4. roll stats: pick one peak (70-100), one dump (1-20), fill the other three with rarity-floor bias (floor to 55 range)
5. generate an "inspiration seed" (a random float) — used externally if you want the LLM reaction logic to vary per companion

**companion lifecycle:**
- `roll(userId)` → `{ bones, inspirationSeed }`
- `getCompanion()` → reads soul from config, regenerates bones, returns full `Companion`
- hatching: first time the user runs `/buddy`, generate a name + personality via LLM call, store soul to config

---

### component 3: sprite library

**format:** each species has N frames (usually 3). each frame is a 5-line, 12-char-wide string array. use `{E}` as the placeholder for the eye character.

**hat system:** the hat occupies line 0 (the top line) of the sprite. only inject the hat if line 0 is blank in the current frame — some fidget frames use that line for effects. this prevents height oscillation.

**blink state:** add a blink frame (frame = -1) that replaces the eye character with `-`. implement this as a rendering transform, not a stored frame.

**rendering function:**
```
renderSprite(bones, frame) → string[]
  1. get frame lines for species
  2. replace {E} with bones.eyes
  3. if frame 0 is blank, inject hat string at line 0
  4. if frame === -1, replace eyes with -
  5. if hat line is empty across all frames, remove it
  6. return lines
```

also implement `renderFace(bones)` — a compact 1-line representation (e.g., `(·>` for duck) used when the terminal is too narrow for full sprite.

---

### component 4: react/ink component

**animation loop:**
- `useEffect` with 500ms interval, incrementing a tick counter
- tick drives frame selection and speech bubble fade logic

**idle sequence:**
- mostly frame 0 (rest)
- occasionally frames 1, 2 (fidget)
- rarely frame -1 (blink)
- example pattern: `[0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, -1, 0, ...]`

**states:**
- **idle**: follow idle sequence
- **reacting**: cycle all frames fast, show speech bubble
- **petting**: 2.5s heart overlay animation (`♥ ♡ ❤` etc), then return to idle

**speech bubble:**
- max 30 chars wide, wrap text
- visible for ~20 ticks (~10s)
- fade starts at tick 14 (dims color for last 6 ticks)
- pull text from global app state: `companionReaction`

**responsive layout:**
- narrow mode (<100 cols): show `renderFace()` inline + truncated quip on one line
- full mode: full sprite + floating bubble + stats card

**stats card:**
- show all 5 stats with horizontal bar visualization
- color-code by value (green/yellow/red or similar)

**global state reads:**
- `companionReaction: string | undefined`
- `petAt: number | undefined` (timestamp)
- `companionVisible: boolean`

---

### component 5: system prompt integration

inject a small markdown block into the first user message of each session (as an attachment or prepended context):

```
you're working alongside [name], a [species] companion watching this conversation.
you're not [name] — it's a separate watcher.
don't acknowledge it unless the user addresses it directly by name.
if they do, respond with one short line as if you were [name].
otherwise stay out of it.
```

skip injection if: companion is muted, companion hasn't hatched yet, or intro was already shown this session.

---

### component 6: command handler

`/buddy` with subcommands:

- `/buddy` — show companion info card (name, species, rarity, stats)
- `/buddy pet` — trigger pet animation (set `petAt` in global state, companion cheers)
- `/buddy mute` — toggle `companionMuted` in config
- `/buddy reset` — clear stored soul from config (forces re-hatch next run)

also: scan prompt input text for `/buddy` mentions and highlight them (rainbow or accent color) — this makes the command discoverable without cluttering the UI.

---

### observer pattern (bonus — reaction quips)

after each LLM turn completes, call an async observer:

```
fireCompanionObserver(messages, companion) → string | null
```

this does a lightweight LLM call with the companion's personality and recent messages, asking for a 1-2 sentence quip in the companion's voice. update `companionReaction` in global state. the UI polls this state and shows the bubble automatically.

keep it brief and non-blocking. the companion should feel ambient, not intrusive.

---

### integration checklist

- [ ] guard entire module behind a feature flag (`feature('BUDDY')` or env var)
- [ ] companion bones never written to disk
- [ ] companion soul (name, personality, hatchedAt) stored in user config
- [ ] roll result cached per userId to avoid recomputing on every render tick
- [ ] animation loop uses `clearInterval` cleanup to avoid memory leaks
- [ ] narrow terminal detection gates full vs. compact rendering
- [ ] system prompt attachment skipped if muted or not yet hatched
- [ ] `/buddy` command registered conditionally on feature flag

---

### build order recommendation

1. types.ts — no deps, establishes everything else
2. companion.ts — depends on types only
3. sprites.ts — depends on types only (can be done in parallel with companion.ts)
4. prompt.ts — depends on types + companion, short
5. CompanionSprite.tsx — depends on all of the above + app state
6. command handler — depends on companion + app state
7. useBuddyNotification.tsx — thin wrapper, last
