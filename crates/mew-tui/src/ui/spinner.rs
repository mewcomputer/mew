#![allow(dead_code, non_snake_case)]
const UNICODE_SPINNER_FRAMES: &str = "⡀⡁⡂⡃⡄⡅⡆⡇⡈⡉⡊⡋⡌⡍⡎⡏⡐⡑⡒⡓⡔⡕⡖⡗⡘⡙⡚⡛⡜⡝⡞⡟⡠⡡⡢⡣⡤⡥⡦⡧⡨⡩⡪⡫⡬⡭⡮⡯⡰⡱⡲⡳⡴⡵⡶⡷⡸⡹⡺⡻⡼⡽⡾⡿⢀⢁⢂⢃⢄⢅⢆⢇⢈⢉⢊⢋⢌⢍⢎⢏⢐⢑⢒⢓⢔⢕⢖⢗⢘⢙⢚⢛⢜⢝⢞⢟⢠⢡⢢⢣⢤⢥⢦⢧⢨⢩⢪⢫⢬⢭⢮⢯⢰⢱⢲⢳⢴⢵⢶⢷⢸⢹⢺⢻⢼⢽⢾⢿⣀⣁⣂⣃⣄⣅⣆⣇⣈⣉⣊⣋⣌⣍⣎⣏⣐⣑⣒⣓⣔⣕⣖⣗⣘⣙⣚⣛⣜⣝⣞⣟⣠⣡⣢⣣⣤⣥⣦⣧⣨⣩⣪⣫⣬⣭⣮⣯⣰⣱⣲⣳⣴⣵⣶⣷⣸⣹⣺⣻⣼⣽⣾⣿";
const CLAUDIUS_SPINNER_FRAMES: &str = "·✻✽✶✳✢";
const BALL_SPINNER_FRAMES: &str = "⠁⠂⠄⡀_⢀⠠⠐⠈";

enum SpinnerKind {
    Unicode,
    Claudius,
    Ball,
}

pub struct Spinner {
    kind: SpinnerKind,
    frame_index: usize,
    isRandom: bool,
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            kind: SpinnerKind::Unicode,
            frame_index: 0,
            isRandom: false,
        }
    }

    pub fn next_frame(&mut self) -> char {
        let spinner_frames = match self.kind {
            SpinnerKind::Unicode => UNICODE_SPINNER_FRAMES,
            SpinnerKind::Claudius => CLAUDIUS_SPINNER_FRAMES,
            SpinnerKind::Ball => BALL_SPINNER_FRAMES,
        };
        if self.isRandom {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let random_index = rng.gen_range(0..spinner_frames.chars().count());
            return spinner_frames.chars().nth(random_index).unwrap_or(' ');
        }
        let frame = spinner_frames.chars().nth(self.frame_index).unwrap_or(' ');
        self.frame_index = (self.frame_index + 1) % spinner_frames.chars().count();
        frame
    }
}

/// The default spinner frame sequence (braille dots). 10 frames — short
/// enough for a ~160ms full cycle at 16ms/tick (~62fps), which feels
/// snappy and alive without being distracting. The classic braille
/// spinner used by most CLI tools.
pub fn spinner_frames() -> &'static str {
    "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
}

// Ratatui component that displays a spinner animation, and automatically starts. The spinner should be non-blocking and update on each tick of the UI.
