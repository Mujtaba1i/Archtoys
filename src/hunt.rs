//! The bug hunt (v0.8.0): hostile input for the "survives" tests next to
//! each function that reads something from outside (typed text, files,
//! downloads, screenshots). The tests check that nothing panics: a panic in
//! a release build closes Archtoys (panic = "abort").

/// A small, repeatable random number generator (xorshift), so a failure can
/// be run again exactly.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

/// Text no one should type, and some anyone might: empty, spaces, huge,
/// other scripts, control characters, numbers at and past every limit,
/// almost-right color codes, and random mixes of all of them.
pub(crate) fn texts() -> Vec<String> {
    let mut out: Vec<String> = [
        "",
        " ",
        "\t\n\r",
        "\0",
        "#",
        "##",
        "#ffff",
        "#gggggg",
        "#12345678901234567890",
        "rgb(",
        "rgb()",
        "rgb(,,)",
        "rgb(255,255,255,255,255)",
        "rgb(-1,-1,-1)",
        "rgb(999999999999999999999,0,0)",
        "rgb(1e309,0,0)",
        "rgb(NaN,inf,-inf)",
        "hsl(360,100%,100%)",
        "hsl(-720, -5%, 500%)",
        "hsla(1,2,3,4,5)",
        "cmyk(100%,100%,100%,100%)",
        "cmyk(0,0,0)",
        "oklch(1 1 1 1)",
        "oklch(NaN NaN NaN)",
        "oklab(-1e30 1e30 0)",
        "hwb(0 200% 200%)",
        "%%%%",
        "((((((((",
        "))))",
        ",,,,,",
        "+",
        "++",
        "Ctrl+",
        "+C",
        "Ctrl++C",
        "Ctrl+Shift+Alt+Super+Meta+F99",
        "F0",
        "F255",
        "Key.Escape",
        "ÄÖÜ ß é",
        "日本語のテキスト",
        "مرحبا بالعالم",
        "שלום",
        "🎨🌈🔴",
        "\u{202e}reversed",
        "\u{fffd}\u{feff}",
        "a\u{0301}\u{0301}\u{0301}\u{0301}",
        "red red red red red red red red red red red red red red",
        "dark light pale deep red",
        "xqzvw",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    out.push("a".repeat(100_000));
    out.push("#".repeat(10_000));
    out.push("9".repeat(5_000));
    out.push("(".repeat(5_000));
    out.push("é".repeat(20_000));
    // random mixes: printable, punctuation, digits and some multibyte characters
    let pieces = ["#", "rgb(", ")", ",", "%", " ", "-", ".", "e", "1", "255", "0", "ff", "red", "+", "Ctrl", "日", "é", "\n", "NaN"];
    let mut rng = Rng::new(0x5eed);
    for _ in 0..2_000 {
        let n = rng.below(12) as usize;
        out.push((0..n).map(|_| pieces[rng.below(pieces.len() as u64) as usize]).collect());
    }
    for _ in 0..300 {
        let n = rng.below(40) as usize;
        out.push(String::from_utf8_lossy(&rng.bytes(n)).into_owned());
    }
    out
}

/// Files that aren't what they claim: empty, one byte, random bytes, a
/// real format's start followed by garbage, cut in the middle.
pub(crate) fn files() -> Vec<Vec<u8>> {
    let mut rng = Rng::new(0xf11e);
    let mut out = vec![
        vec![],
        vec![0],
        b"{".to_vec(),
        b"null".to_vec(),
        b"[]".to_vec(),
        b"\x89PNG\r\n\x1a\n".to_vec(),
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\xff\xff\xff\xff\xff\xff\xff\xff\x08\x06\0\0\0".to_vec(),
        b"\x1f\x8b\x08\0\0\0\0\0".to_vec(),
        b"ATFI".to_vec(),
    ];
    for len in [1, 7, 64, 1000, 100_000] {
        out.push(rng.bytes(len));
    }
    out
}
