#[cfg(test)]
mod tests {
    use super::*;
    use resonance_types::{ingame_pipe::ErrorCode, SendTextError};

    fn proc(pid: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            exe: exe.to_string(),
        }
    }

    #[test]
    fn game_exe_names_match_by_prefix_and_extension() {
        for name in ["StarSEA.exe", "StarSEA_STEAM.exe", "StarASIA.exe", "starasia.EXE"] {
            assert!(is_game_exe(name), "{name}");
        }
        for name in ["StarSEA", "notepad.exe", "MyStarSEA.exe", ""] {
            assert!(!is_game_exe(name), "{name}");
        }
    }

    #[test]
    fn pick_game_with_no_match_does_not_ask_for_port_owners() {
        let list = [proc(1, "notepad.exe")];
        let got = pick_game(&list, || panic!("asked"), |_| false);
        assert_eq!(got, Err(GameError::NotRunning));
        assert_eq!(pick_game(&[], || panic!("asked"), |_| false), Err(GameError::NotRunning));
    }

    #[test]
    fn pick_game_with_one_match_returns_it_without_asking() {
        let list = [proc(5, "notepad.exe"), proc(10, "StarSEA.exe")];
        assert_eq!(pick_game(&list, || panic!("asked"), |_| false), Ok(10));
    }

    #[test]
    fn pick_game_with_two_matches_takes_the_port_owner() {
        let list = [proc(10, "StarSEA.exe"), proc(20, "StarASIA.exe")];
        assert_eq!(pick_game(&list, || vec![20], |_| false), Ok(20));
        assert_eq!(pick_game(&list, Vec::new, |_| false), Err(GameError::Several));
        assert_eq!(
            pick_game(&list, || vec![10, 20], |_| false),
            Err(GameError::Several)
        );
        // an owner that is no game process does not count
        assert_eq!(pick_game(&list, || vec![99, 10], |_| false), Ok(10));
    }

    #[test]
    fn pick_game_refuses_a_32_bit_game() {
        let list = [proc(10, "StarSEA.exe")];
        assert_eq!(
            pick_game(&list, || panic!("asked"), |pid| pid == 10),
            Err(GameError::Is32Bit)
        );
    }

    #[test]
    fn pick_game_ignores_non_game_processes_with_equal_pids() {
        let list = [proc(10, "notepad.exe"), proc(20, "StarSEA.exe")];
        assert_eq!(pick_game(&list, || vec![10], |_| false), Ok(20));
    }

    #[test]
    fn remote_port_reads_the_low_16_bits_in_network_order() {
        assert_eq!(remote_port(0x8B13), 5003);
        assert_eq!(remote_port(0xFFFF_8B13), 5003);
        assert_eq!(remote_port(0x5000), 80);
    }

    #[test]
    fn sha256_hex_is_lower_case_hex() {
        assert_eq!(
            sha256_hex(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(&b""[..]).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    const PIN: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn check_pin_compares_hex_case_insensitively() {
        assert_eq!(check_pin(Some(PIN), PIN), Ok(()));
        assert_eq!(check_pin(Some(&PIN.to_uppercase()), PIN), Ok(()));
        let other = PIN.replacen('b', "c", 1);
        assert_eq!(check_pin(Some(&other), PIN), Err(PinError::Mismatch));
    }

    #[test]
    fn check_pin_without_a_usable_pin_is_no_pin() {
        assert_eq!(check_pin(None, PIN), Err(PinError::NoPin));
        assert_eq!(check_pin(Some(""), PIN), Err(PinError::NoPin));
        assert_eq!(check_pin(Some(&PIN[..63]), PIN), Err(PinError::NoPin));
        assert_eq!(check_pin(Some(&PIN.replace('a', "z")), PIN), Err(PinError::NoPin));
    }

    #[test]
    fn the_pipe_sddl_is_exact() {
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;S-1-5-21-1-2-3-1001)S:(ML;;NW;;;ME)"
        );
    }

    #[test]
    fn code_text_is_the_ten_sentences_in_order() {
        let want = [
            "게임 내 전송 모듈과 통신하지 못했습니다. 앱을 다시 시작해 주세요.",
            "이 채널로는 보낼 수 없습니다.",
            "보낼 내용이 없습니다.",
            "메시지가 너무 깁니다. 140자 이하로 줄여 주세요.",
            "게임 채팅 서버에 연결되어 있지 않습니다. 로그인한 뒤 다시 시도해 주세요.",
            "게임이 메시지를 보내지 못했습니다.",
            "이전 메시지를 보내는 중입니다. 잠시 후 다시 시도해 주세요.",
            "게임이 응답하지 않아 보내지 않았습니다. 로딩이 끝난 뒤 다시 시도해 주세요.",
            "이 게임 버전에서는 채팅 보내기를 지원하지 않습니다. 앱 업데이트를 기다려 주세요.",
            "게임 내 전송 모듈의 버전이 맞지 않습니다. 앱을 다시 시작해 주세요.",
        ];
        let got: Vec<&str> = ErrorCode::ALL.into_iter().map(code_text).collect();
        assert_eq!(got, want);
        for (i, a) in got.iter().enumerate() {
            assert!(!a.is_empty() && !a.contains('\n'));
            assert!(got[i + 1..].iter().all(|b| a != b));
        }
        for code in ErrorCode::ALL {
            assert_eq!(SendError::Dll(code).to_string(), code_text(code));
        }
    }

    #[test]
    fn every_send_error_displays_its_sentence() {
        let table = [
            (SendError::SettingOff, "게임 내 채팅 보내기가 꺼져 있습니다. 설정에서 켜 주세요."),
            (SendError::Game(GameError::NotRunning), "게임이 실행 중이 아닙니다."),
            (SendError::Game(GameError::Several), "게임이 둘 이상 실행 중입니다."),
            (SendError::Game(GameError::Is32Bit), "32비트 게임은 지원하지 않습니다."),
            (SendError::NoPin, "이 앱에는 게임 내 전송 모듈이 들어 있지 않습니다."),
            (
                SendError::DllMissing,
                "게임 내 전송 모듈 파일을 찾을 수 없습니다. 앱을 다시 설치해 주세요.",
            ),
            (
                SendError::PinMismatch,
                "게임 내 전송 모듈 파일이 변경되어 불러오지 않았습니다. 앱을 다시 설치해 주세요.",
            ),
            (SendError::OpenFailed, "게임 프로세스에 접근하지 못했습니다."),
            (SendError::LoadFailed, "게임에 전송 모듈을 불러오지 못했습니다."),
            (
                SendError::PipeCreate,
                "게임 내 전송 모듈과 통신할 수 없습니다. 앱을 다시 시작해 주세요.",
            ),
            (SendError::NoConnect, "게임 내 전송 모듈이 연결되지 않았습니다."),
            (
                SendError::Broken,
                "게임 내 전송 모듈이 응답하지 않습니다. 설정에서 껐다 켜 주세요.",
            ),
            (
                SendError::StaleModule,
                "이전 전송 모듈이 아직 게임에 남아 있습니다. 잠시 후 다시 시도해 주세요.",
            ),
        ];
        for (err, sentence) in table {
            assert_eq!(err.to_string(), sentence);
            assert!(!sentence.contains('\n'));
        }
        assert_eq!(
            SendError::Text(SendTextError::Empty).to_string(),
            SendTextError::Empty.to_string()
        );
    }
}
