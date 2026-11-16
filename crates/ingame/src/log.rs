#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "resonance-ingame-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(dir: &std::path::Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn line_format() {
        assert_eq!(LOG_CAP_BYTES, 1024 * 1024);
        assert_eq!(
            format_line(1_700_000_000_123, Level::Info, "hello"),
            "1700000000.123 INFO hello\n"
        );
        assert_eq!(
            format_line(1_700_000_000_005, Level::Warn, "x"),
            "1700000000.005 WARN x\n"
        );
        assert_eq!(format_line(0, Level::Error, "x"), "0.000 ERROR x\n");
    }

    #[test]
    fn newlines_become_spaces_and_long_messages_are_cut() {
        let line = format_line(1, Level::Info, "a\nb\r\nc");
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with("a b  c\n"));
        let line = format_line(1, Level::Info, &"x".repeat(5000));
        let msg = line.trim_end().strip_prefix("0.001 INFO ").unwrap();
        assert_eq!(msg.len(), MAX_MESSAGE_BYTES);
        // A cut never splits a character.
        let line = format_line(1, Level::Info, &"가".repeat(500));
        let msg = line.trim_end().strip_prefix("0.001 INFO ").unwrap();
        assert!(msg.len() <= MAX_MESSAGE_BYTES && msg.chars().all(|c| c == '가'));
    }

    #[test]
    fn appends_under_the_cap() {
        let dir = temp_dir("append");
        let mut log = RotatingLog::open(&dir, 100);
        log.write(1_700_000_000_123, Level::Info, "one");
        log.write(1_700_000_000_124, Level::Info, "two");
        assert_eq!(
            read(&dir, "ingame.log"),
            "1700000000.123 INFO one\n1700000000.124 INFO two\n"
        );
        assert!(!dir.join("ingame.log.1").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotates_once_and_replaces_the_old_one() {
        let dir = temp_dir("rotate");
        let mut log = RotatingLog::open(&dir, 100);
        // Each line is 24 bytes: four fit, the fifth rotates.
        for i in 0..5 {
            log.write(1_700_000_000_000 + i, Level::Info, "a");
        }
        let first = format_line(1_700_000_000_004, Level::Info, "a");
        assert_eq!(read(&dir, "ingame.log"), first);
        assert_eq!(read(&dir, "ingame.log.1").lines().count(), 4);
        // A second rotation replaces .1; there is no .2.
        for i in 5..10 {
            log.write(1_700_000_000_000 + i, Level::Info, "a");
        }
        assert!(!dir.join("ingame.log.2").exists());
        assert!(read(&dir, "ingame.log.1").contains("1700000000.005"));
        assert!(!read(&dir, "ingame.log.1").contains("1700000000.000"));
        for name in ["ingame.log", "ingame.log.1"] {
            assert!(fs::metadata(dir.join(name)).unwrap().len() <= 100);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reopening_continues_the_size_count() {
        let dir = temp_dir("reopen");
        let mut log = RotatingLog::open(&dir, 100);
        for i in 0..3 {
            log.write(1_700_000_000_000 + i, Level::Info, "a");
        }
        drop(log);
        let mut log = RotatingLog::open(&dir, 100);
        log.write(1_700_000_000_003, Level::Info, "a"); // 4th line: 96 bytes
        assert!(!dir.join("ingame.log.1").exists());
        log.write(1_700_000_000_004, Level::Info, "a"); // would pass the cap
        assert!(dir.join("ingame.log.1").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unwritable_directory_gives_an_inert_log() {
        let dir = temp_dir("inert");
        let file = dir.join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        let mut log = RotatingLog::open(&file, 100);
        log.write(1, Level::Info, "nothing happens");
        let mut log = RotatingLog::inert();
        log.write(1, Level::Info, "nothing happens");
        fs::remove_dir_all(&dir).unwrap();
    }
}
