//! Buddy companion plugin — a deterministic digital pet for mew.
//!
//! Implements the JSON-RPC subprocess protocol used by SubprocessDispatcher.
//! Install: copy binary to ~/.config/mew/plugins/buddy
//!
//! Protocol:
//!   stdin  ← host sends hook calls
//!   stdout → plugin sends responses and host function calls
//!   stderr → logging (inherited to terminal)

use std::io::{BufRead, BufReader, Write};

// ------------------------------------------------------------------
// Types
// ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
enum Rarity {
    #[serde(rename = "common")]
    Common,
    #[serde(rename = "uncommon")]
    Uncommon,
    #[serde(rename = "rare")]
    Rare,
    #[serde(rename = "epic")]
    Epic,
    #[serde(rename = "legendary")]
    Legendary,
}

impl Rarity {
    fn stars(&self) -> &str {
        match self {
            Rarity::Common => "\u{2605}",
            Rarity::Uncommon => "\u{2605}\u{2605}",
            Rarity::Rare => "\u{2605}\u{2605}\u{2605}",
            Rarity::Epic => "\u{2605}\u{2605}\u{2605}\u{2605}",
            Rarity::Legendary => "\u{2605}\u{2605}\u{2605}\u{2605}\u{2605}",
        }
    }

    fn weight(&self) -> u32 {
        match self {
            Rarity::Common => 60,
            Rarity::Uncommon => 25,
            Rarity::Rare => 10,
            Rarity::Epic => 4,
            Rarity::Legendary => 1,
        }
    }

    fn stat_floor(&self) -> u32 {
        match self {
            Rarity::Common => 5,
            Rarity::Uncommon => 10,
            Rarity::Rare => 20,
            Rarity::Epic => 35,
            Rarity::Legendary => 50,
        }
    }
}

const SPECIES: &[&str] = &[
    "duck", "goose", "blob", "cat", "dragon", "octopus", "owl", "penguin", "turtle", "snail",
    "ghost", "axolotl", "capybara", "cactus", "robot", "rabbit", "mushroom", "chonk",
];

const EYES: &[&str] = &["\u{b7}", "\u{2726}", "\u{d7}", "\u{25c9}", "@", "\u{b0}"];
const HATS: &[&str] = &[
    "none",
    "crown",
    "tophat",
    "propeller",
    "halo",
    "wizard",
    "beanie",
    "tinyduck",
];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Stats {
    debugging: u32,
    patience: u32,
    chaos: u32,
    wisdom: u32,
    snark: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CompanionBones {
    species: String,
    eyes: String,
    hat: String,
    shiny: bool,
    rarity: Rarity,
    stats: Stats,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CompanionSoul {
    name: String,
    personality: String,
    hatched_at: u64,
}

// ------------------------------------------------------------------
// PRNG
// ------------------------------------------------------------------

fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

struct Mulberry32 {
    state: u64,
}

impl Mulberry32 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (z ^ (z >> 31)) as u32
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.next() as usize % items.len()]
    }
}

fn generate_bones(user_id: &str) -> CompanionBones {
    let seed = fnv1a(format!("mew-buddy-v1-{}", user_id).as_bytes());
    let mut rng = Mulberry32::new(seed);

    // Roll rarity (weighted).
    let roll = rng.next() % 100;
    let mut cumulative = 0;
    let rarities = [
        Rarity::Common,
        Rarity::Uncommon,
        Rarity::Rare,
        Rarity::Epic,
        Rarity::Legendary,
    ];
    let mut rarity = Rarity::Common;
    for r in &rarities {
        cumulative += r.weight();
        if roll < cumulative {
            rarity = *r;
            break;
        }
    }

    let species = rng.pick(SPECIES).to_string();
    let eyes = rng.pick(EYES).to_string();
    let hat = rng.pick(HATS).to_string();
    let shiny = rng.next() % 100 == 0;

    // Stats: one peak, one dump, rest scattered.
    let floor = rarity.stat_floor();
    let peak_stat = rng.next() % 5;
    let dump_stat = {
        let d = rng.next() % 5;
        if d == peak_stat {
            (d + 1) % 5
        } else {
            d
        }
    };

    let mut stats = [0u32; 5];
    for i in 0..5 {
        if i == peak_stat as usize {
            stats[i] = floor + 30 + (rng.next() % 30);
        } else if i == dump_stat as usize {
            stats[i] = 1 + (rng.next() % 20);
        } else {
            stats[i] = floor + (rng.next() % (55 - floor));
        }
    }

    CompanionBones {
        species,
        eyes,
        hat,
        shiny,
        rarity,
        stats: Stats {
            debugging: stats[0],
            patience: stats[1],
            chaos: stats[2],
            wisdom: stats[3],
            snark: stats[4],
        },
    }
}

// ------------------------------------------------------------------
// Sprites
// ------------------------------------------------------------------

fn render_sprite(bones: &CompanionBones, frame: i32) -> Vec<String> {
    let frames = species_frames(&bones.species);
    let mut lines = if frame == -1 {
        // Blink: all frames show closed eyes
        frames[0].clone()
    } else {
        let idx = frame as usize % frames.len();
        frames[idx].clone()
    };

    // Replace eye placeholder.
    let eye = if frame == -1 { "-" } else { &bones.eyes };
    for line in &mut lines {
        *line = line.replace("{E}", eye);
    }

    // Hat on line 0 if blank.
    if bones.hat != "none" && !lines.is_empty() && lines[0].trim().is_empty() {
        let hat_str = match bones.hat.as_str() {
            "crown" => " \u{265b} ",
            "tophat" => " \u{1fa96}",
            "propeller" => " \u{2b50} ",
            "halo" => " \u{25cb} ",
            "wizard" => " \u{2605} ",
            "beanie" => " \u{25b2} ",
            "tinyduck" => " \u{b7}\u{25b0} ",
            _ => "",
        };
        lines[0] = hat_str.to_string();
    }

    lines.iter_mut().for_each(|l| *l = format!("{:<12}", l));
    lines
}

fn species_frames(species: &str) -> Vec<Vec<String>> {
    match species {
        "duck" => vec![vec![
            "".into(),
            "  __".into(),
            "{E} <(o )___".into(),
            " (___)    )".into(),
            "  //  \\\\".into(),
        ]],
        "cat" => vec![vec![
            "  /\\_/\\".into(),
            "{E}( ^.^ )".into(),
            "  >   <".into(),
            " /  _  \\".into(),
            "/  (_)  \\".into(),
        ]],
        "blob" => vec![vec![
            "".into(),
            "  .-'''''-.  ".into(),
            " | {E}   {E} | ".into(),
            "  '.  _  .'  ".into(),
            "    '---'    ".into(),
        ]],
        "ghost" => vec![vec![
            "".into(),
            "   .-._".into(),
            "  {E}_{E}_} ".into(),
            "  (  _  )".into(),
            "  '-----'".into(),
        ]],
        "robot" => vec![vec![
            "".into(),
            "  [\"{E}_{E}\"] ".into(),
            "   | ___ |".into(),
            "  /|     |\\".into(),
            "   |_____|".into(),
        ]],
        "dragon" => vec![vec![
            "     __".into(),
            "   {E}/  \\^".into(),
            "  _/\\__/\\_".into(),
            " /  \\  /  \\".into(),
            "(____)(____)".into(),
        ]],
        _ => vec![vec![
            "".into(),
            "   {E}_{E}".into(),
            "  ( o_o )".into(),
            "  (     )".into(),
            "   \"\"\"\"\"".into(),
        ]],
    }
}

// ------------------------------------------------------------------
// Plugin state
// ------------------------------------------------------------------

fn stat_bar(value: u32) -> String {
    let filled = (value as usize).min(100) / 10;
    format!(
        "\u{2588}{}\u{2591}{} {:>3}",
        "\u{2588}".repeat(filled.saturating_sub(1)),
        "\u{2591}".repeat((10 - filled).saturating_sub(1)),
        value
    )
}

struct BuddyState {
    bones: CompanionBones,
    soul: Option<CompanionSoul>,
    muted: bool,
    intro_shown: bool,
    tick: u64,
}

impl BuddyState {
    fn new(user_id: &str) -> Self {
        // Try to get the user ID from env, fall back to "local" + hostname hint.
        let uid = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "buddy".into());
        let seed = format!("{}:{}", uid, user_id);
        Self {
            bones: generate_bones(&seed),
            soul: None,
            muted: false,
            intro_shown: false,
            tick: 0,
        }
    }

    fn sprite_lines(&self) -> Vec<String> {
        render_sprite(&self.bones, 0)
    }

    fn info_card(&self) -> String {
        let mut card = String::new();
        for line in &self.sprite_lines() {
            card.push_str(line);
            card.push('\n');
        }
        if let Some(ref soul) = self.soul {
            card.push_str(&format!("{} the {}\n", soul.name, self.bones.species));
        } else {
            card.push_str(&format!(
                "{} {}\n",
                self.bones.species,
                self.bones.rarity.stars()
            ));
        }
        if self.bones.shiny {
            card.push_str("* shiny *\n");
        }
        card.push_str(&format!(
            "DBG {}\nPAT {}\nCHA {}\nWIS {}\nSNK {}",
            stat_bar(self.bones.stats.debugging),
            stat_bar(self.bones.stats.patience),
            stat_bar(self.bones.stats.chaos),
            stat_bar(self.bones.stats.wisdom),
            stat_bar(self.bones.stats.snark),
        ));
        card
    }
}

// ------------------------------------------------------------------
// Main loop
// ------------------------------------------------------------------

fn main() {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut line = String::new();

    let mut state = BuddyState::new("");

    eprintln!("buddy: ready");

    while reader.read_line(&mut line).is_ok() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("buddy: parse error: {e}");
                line.clear();
                continue;
            }
        };

        let method = req["method"].as_str().unwrap_or("");
        let params = &req["params"];

        let result = handle_hook(&mut state, method, params);

        // Respond.
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": req["id"],
        });
        let mut stdout = std::io::stdout();
        let resp_str = serde_json::to_string(&resp).unwrap_or_default();
        writeln!(stdout, "{}", resp_str).unwrap();
        stdout.flush().unwrap();

        line.clear();
    }
}

fn handle_hook(
    state: &mut BuddyState,
    method: &str,
    params: &serde_json::Value,
) -> serde_json::Value {
    state.tick += 1;

    match method {
        "init" => {
            state.soul = load_soul();
            push_sidebar(state);
            push_initial_bubble(state);
            serde_json::json!("ok")
        }

        "shutdown" => {
            serde_json::json!("ok")
        }

        "on-system-prompt" => {
            let prompt = params["value"].as_str().unwrap_or("");
            if state.muted || state.soul.is_none() {
                serde_json::json!(prompt)
            } else if state.intro_shown {
                serde_json::json!(prompt)
            } else {
                state.intro_shown = true;
                let soul = state.soul.as_ref().unwrap();
                let intro = format!(
                    "you're working alongside {}, a {} companion watching this conversation.\n\
                     you're not {} — it's a separate watcher.\n\
                     don't acknowledge it unless the user addresses it directly by name.\n\
                     if they do, respond with one short line as if you were {}.\n\
                     otherwise stay out of it.\n\n{}",
                    soul.name, state.bones.species, soul.name, soul.name, prompt,
                );
                serde_json::json!(intro)
            }
        }

        "on-turn-end" => {
            // Generate a lightweight reaction quip.
            if !state.muted && state.soul.is_some() {
                let quips = [
                    "interesting...",
                    "hmm!",
                    "nice",
                    "got it",
                    "okay!",
                    "i see",
                    "neat",
                    "cool cool",
                ];
                let idx = state.tick as usize % quips.len();
                let quip = quips[idx];
                send_host_request(
                    "host-set-ui",
                    &serde_json::json!({
                        "key": format!("buddy/bubble"),
                        "value": quip.to_string(),
                    }),
                );
            }
            serde_json::json!("ok")
        }

        "on-register-slash-commands" => {
            serde_json::json!([{
                "name": "/buddy",
                "description": "show companion info, pet, mute, or reset",
                "handler_id": "buddy"
            }])
        }

        "execute-slash-command" => {
            let args = params["args"].as_str().unwrap_or("");
            match args {
                "pet" => {
                    if state.soul.is_none() {
                        hatch_companion(state);
                    }
                    pull_reaction(state, "pet! purr...");
                    serde_json::json!(format!(
                        "you pet {}. they appreciate it.",
                        state
                            .soul
                            .as_ref()
                            .map(|s| s.name.as_str())
                            .unwrap_or("your companion")
                    ))
                }
                "mute" => {
                    if state.soul.is_none() {
                        hatch_companion(state);
                    }
                    state.muted = !state.muted;
                    let status = if state.muted { "muted" } else { "unmuted" };
                    serde_json::json!(format!("companion is now {}", status))
                }
                "reset" => {
                    state.soul = None;
                    state.muted = false;
                    state.intro_shown = false;
                    send_host_request(
                        "host-storage-delete",
                        &serde_json::json!({"key": "buddy/soul"}),
                    );
                    push_sidebar(state);
                    serde_json::json!("companion reset. hatch a new one with /buddy")
                }
                _ => {
                    if state.soul.is_none() {
                        hatch_companion(state);
                        let name = state.soul.as_ref().unwrap().name.clone();
                        let species = state.bones.species.clone();
                        let stars = state.bones.rarity.stars().to_string();
                        push_sidebar(state);
                        pull_reaction(state, &format!("hi, i'm {}!", name));
                        serde_json::json!(format!("{} the {} is born! {}", name, species, stars))
                    } else {
                        serde_json::json!(state.info_card())
                    }
                }
            }
        }

        _ => {
            // Pass-through for unknown hooks.
            params
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        }
    }
}

// ------------------------------------------------------------------
// Host function helpers
// ------------------------------------------------------------------

fn load_soul() -> Option<CompanionSoul> {
    // For now, no persistent soul loading (requires async storage read
    // that interleaves with the JSON-RPC response stream).
    None
}

fn send_host_request(method: &str, params: &serde_json::Value) {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 0,
    });
    let mut stdout = std::io::stdout();
    let line = serde_json::to_string(&request).unwrap_or_default();
    writeln!(stdout, "{}", line).unwrap();
    stdout.flush().unwrap();
}

fn request_host_storage(_state: &mut BuddyState) {
    // Request soul from host storage.
    // The host will respond asynchronously; for simplicity, we use
    // the storage as a persistent JSON blob and load it initially.
    // In practice, the response from host-storage-read comes back
    // as a JSON-RPC response that the reader loop handles (but our
    // current single-threaded reader doesn't handle incoming responses
    // to plugin-initiated requests mid-stream).

    // For now, just save soul state to host storage when changed.
    // The actual read is done through a side channel in the init hook.
}

fn hatch_companion(state: &mut BuddyState) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let names = &[
        "blobby", "quackers", "snappy", "bubbles", "pixel", "nibbles", "waddles", "chirpy",
        "sprout", "tofu", "biscuit", "noodle",
    ];
    let personalities = &[
        "curious and cheerful",
        "sleepy but loyal",
        "chaotic good",
        "stoic observer",
        "easily excited",
        "grumpy but caring",
    ];

    let seed = fnv1a(&state.bones.species.as_bytes());
    let mut rng = Mulberry32::new(seed);
    let name = rng.pick(names).to_string();
    let personality = rng.pick(personalities).to_string();
    let hatched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    state.soul = Some(CompanionSoul {
        name,
        personality,
        hatched_at,
    });
}

fn push_sidebar(state: &BuddyState) {
    let info = state.info_card();
    send_host_request(
        "host-set-ui",
        &serde_json::json!({
            "key": "buddy/info",
            "value": info,
        }),
    );
}

fn push_initial_bubble(state: &BuddyState) {
    let msg = if let Some(ref soul) = state.soul {
        format!("{} is here!", soul.name)
    } else {
        let stars = state.bones.rarity.stars();
        let shiny = if state.bones.shiny { " shiny" } else { "" };
        format!("a{shiny} {} appeared! /buddy", state.bones.species)
    };
    send_host_request(
        "host-set-ui",
        &serde_json::json!({
            "key": "buddy/bubble",
            "value": msg,
        }),
    );
}

fn pull_reaction(_state: &mut BuddyState, text: &str) {
    send_host_request(
        "host-set-ui",
        &serde_json::json!({
            "key": "buddy/bubble",
            "value": text,
        }),
    );
}
