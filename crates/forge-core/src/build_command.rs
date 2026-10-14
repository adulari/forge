//! Recognizes environment provisioning and native build/link invocations — the heuristic the
//! env-fight spend cap keys on.

const PHRASES: &[&str] = &[
    "pip install",
    "pip3 install",
    "-m pip",
    "-m ensurepip",
    "-m venv",
    "virtualenv",
    "uv venv",
    "uv pip",
    "apt-get install",
    "apt install",
    "conda install",
    "conda create",
    "yum install",
    "apk add",
    "setup.py build",
    "python setup.py",
    "./configure",
    "cmake",
    "pyenv",
];

const TOOLS: &[&str] = &[
    "make",
    "gmake",
    "meson",
    "ninja",
    "ninja-build",
    "cc",
    "gcc",
    "g++",
    "c++",
    "clang",
    "clang++",
    "clang-cl",
    "ld",
    "lld",
    "ld.lld",
    "ld.bfd",
    "ld.gold",
    "mold",
];

/// Names that also appear behind a cross-compiler triple (`aarch64-linux-gnu-gcc`).
const TRIPLE_PREFIXED: &[&str] = &["gcc", "g++", "c++", "clang", "clang++"];

/// Commands that run their argument as the real command.
const WRAPPERS: &[&str] = &[
    "sudo", "env", "ccache", "sccache", "distcc", "icecc", "time", "nice", "nohup", "command",
    "exec", "xargs", "stdbuf", "setsid", "xcrun", "bash", "sh", "zsh", "bear", "-c",
];

/// Whether a shell command looks like environment provisioning or a native build. Phrase patterns
/// are substring matches on the whitespace-normalized, lowercased command; compilers, linkers and
/// build drivers are matched as the WHOLE command word of a segment (after env assignments and
/// wrappers like `ccache`/`sudo`), by basename with any version suffix removed, so `make` doesn't
/// fire on `cmake`/`makemigrations` and `cc` doesn't fire on `accept`.
pub(crate) fn is_env_setup_command(cmd: &str) -> bool {
    let c = cmd
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if PHRASES.iter().any(|p| c.contains(p)) {
        return true;
    }
    c.split(['&', '|', ';', '\n']).any(segment_runs_build_tool)
}

fn segment_runs_build_tool(segment: &str) -> bool {
    let mut words = segment
        .split(' ')
        .map(|w| w.trim_matches(|ch| matches!(ch, '"' | '\'' | '(' | ')' | '{' | '}' | '`')))
        .filter(|w| !w.is_empty())
        .skip_while(|w| is_env_assignment(w));
    while let Some(w) = words.next() {
        let name = command_name(w);
        if name == "zig" {
            return matches!(words.next(), Some("cc" | "c++" | "build"));
        }
        if is_build_tool(&name) {
            return true;
        }
        let wrapper = WRAPPERS.contains(&name.as_str()) || (w.starts_with('-') && name != w);
        if !wrapper && !w.starts_with('-') && !is_env_assignment(w) {
            return false;
        }
    }
    false
}

fn is_env_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && !k.starts_with(|ch: char| ch.is_ascii_digit())
                && k.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        }
        None => false,
    }
}

fn command_name(word: &str) -> String {
    let base = word.rsplit('/').next().unwrap_or(word);
    base.strip_suffix(".exe").unwrap_or(base).to_string()
}

fn strip_version(name: &str) -> &str {
    match name.rsplit_once('-') {
        Some((head, tail))
            if !head.is_empty()
                && tail.starts_with(|ch: char| ch.is_ascii_digit())
                && tail.chars().all(|ch| ch.is_ascii_digit() || ch == '.') =>
        {
            head
        }
        _ => name,
    }
}

fn is_build_tool(name: &str) -> bool {
    let name = strip_version(name);
    TOOLS.contains(&name)
        || TRIPLE_PREFIXED
            .iter()
            .any(|t| name.strip_suffix(t).is_some_and(|p| p.ends_with('-')))
}

#[cfg(test)]
mod tests {
    use super::is_env_setup_command;

    #[test]
    fn compilers_linkers_and_wrappers_count() {
        for c in [
            "gcc -c a.c",
            "g++ -shared foo.o -o foo.so",
            "cc -fPIC -c wcslib.c",
            "c++ -O2 a.cpp",
            "clang -c a.c -o a.o",
            "clang++ -std=c++20 a.cpp",
            "clang-19 -c a.c",
            "clang++-19 a.cpp",
            "gcc-14 -c a.c",
            "g++-14 a.cpp",
            "/usr/bin/clang -c a.c",
            "/usr/lib/llvm-19/bin/clang++ a.cpp",
            "./toolchain/bin/gcc-14.2 a.c",
            "x86_64-linux-gnu-gcc-14 a.c",
            "aarch64-linux-gnu-g++ a.cpp",
            "CC=clang CFLAGS=-O2 clang -c a.c",
            "FOO=1 /usr/bin/g++ a.cpp",
            "env CC=gcc clang a.c",
            "ccache clang -c a.c",
            "sccache g++ -c a.cpp",
            "ccache /usr/bin/clang++-19 -c a.cpp",
            "sudo -E ccache gcc a.c",
            "time clang a.c",
            "cd build && clang++ a.cpp",
            "(cd build; clang a.c)",
            "bash -c \"clang a.c\"",
            "sh -c 'cd x && gcc a.c'",
            "xcrun clang a.c",
            "ld -o out a.o",
            "ld.lld -o out a.o",
            "lld -flavor gnu a.o",
            "mold -run make",
            "zig cc a.c",
            "zig c++ a.cpp",
            "make -j4",
            "gmake",
            "ninja -C build",
            "meson compile -C build",
            "cmake --build build",
            "python setup.py build_ext --inplace",
            "pip install numpy",
        ] {
            assert!(is_env_setup_command(c), "{c} must count");
        }
    }

    #[test]
    fn unrelated_commands_do_not_count() {
        for c in [
            "pytest tests/test_concat.py",
            "git status",
            "cargo build",
            "cat requirements.txt",
            "python manage.py makemigrations",
            "cat accumulator.py",
            "grep -rn cc_email .",
            "cat /usr/bin/clang",
            "echo clang",
            "ls gcc-14",
            "which gcc",
            "zig fmt a.zig",
            "my-clang-helper a.c",
            "accept",
            "ccache -s",
        ] {
            assert!(!is_env_setup_command(c), "{c} must NOT count");
        }
    }
}
