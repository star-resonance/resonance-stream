# 🛠️ Resonance Stream 빌드 가이드

**Resonance Stream**을 소스 코드에서 빌드하는 방법입니다. 이 앱은 하나의 Cargo 워크스페이스로,
Tauri 2 백엔드(`src-tauri/`), WebAssembly로 컴파일되는 Leptos 프론트엔드(`src/`),
그리고 플랫폼에 독립적인 두 크레이트(`crates/core`, `crates/types`)로 구성됩니다.

Python 사이드카나 드라이버 SDK 설치는 필요하지 않습니다. 패킷은 Raw Socket으로 읽으며,
번역 엔진(llama.cpp `llama-server`, Vulkan 빌드)과 AI 모델은 앱이 실행 중에 직접 다운로드합니다.

## 📋 1. 사전 요구 사항 (Windows 10/11 x64)

* **Rust** (stable, MSVC 툴체인)와 WebAssembly 타깃:
  ```cmd
  rustup target add wasm32-unknown-unknown
  ```
* **Trunk** (프론트엔드 번들러)와 **Tauri CLI**:
  ```cmd
  cargo install trunk
  cargo install tauri-cli --version "^2"
  ```
* **Node.js**: Trunk의 pre-build 훅이 `npx`로 Tailwind CSS CLI를 실행합니다.
  Node 의존성을 한 번 설치하세요:
  ```cmd
  npm install
  ```
* **just** (릴리스 빌드와 검사 명령에 필요): `cargo install just` 또는 `pip install rust-just`.

## 🚀 2. 개발 모드 실행

패킷 스니퍼는 **관리자 권한**이 필요합니다. 개발 빌드에는 관리자 권한을 요청하는 매니페스트가
포함되지 않으므로, 터미널을 관리자 권한으로 연 뒤 실행하세요:

```cmd
cargo tauri dev
```

`trunk serve`가 1420 포트에서 실행되고, 앱이 여기에 연결됩니다.

## 📦 3. 릴리스 빌드

```cmd
package.bat
```

`package.bat`는 먼저 인게임 DLL을 빌드합니다(`just ingame-build`: C 런타임을 정적으로 링크하고,
DLL이 C 런타임 DLL을 임포트하면 실패하며 임포트 목록을 출력). 그다음
`RESONANCE_INGAME_DLL`·`RESONANCE_INGAME_REQUIRED` 변수(DLL의 해시를 앱에 고정하는 용도)를 지정해 `cargo tauri build`(먼저 `trunk build` 실행)를
실행하고, 생성된 NSIS 설치 파일(`*-setup.exe`)을 `dist\` 폴더로 옮깁니다. 설치 파일에는
DLL이 들어 있지 않습니다. 앱이 게임 안 전송 기능을 켤 때 자기 버전의 DLL을 APPDATA에 내려받습니다.
직접 빌드한 설치 파일은 어떤 릴리스에도 없는 DLL의 해시를 고정하므로 내려받기가 되지 않습니다. 게임 안 전송은 Candidate 빌드로 시험하세요. DLL은 이 방법으로만 빌드합니다. 릴리스 빌드에는 `src-tauri/app.manifest`가 포함되어
설치된 앱은 실행 시 관리자 권한을 요청합니다.

버전 번호는 루트 `Cargo.toml`의 `[workspace.package]` 한 곳에서만 설정합니다.

## ✅ 4. 검사

```cmd
just check
```

포맷 검사, 플랫폼 독립 크레이트의 테스트, 프론트엔드 검사, 그리고 Windows에서는 백엔드 검사와
테스트를 실행합니다. 같은 검사가 모든 push마다 GitHub Actions(`.github/workflows/ci.yml`)에서
실행됩니다. 어떤 검사가 어떤 폴더를 담당하는지는 `CLAUDE.md`를 참고하세요.

## ⚠️ 5. 실행 시 참고 사항

- **최초 실행 시 다운로드:** AI 모델(URL은 프로젝트 메타데이터 gist에서 가져옴), `llama-server`
  Vulkan 빌드(이 저장소의 릴리스에서 다운로드), 사용자 사전. 모두
  `%APPDATA%\com.enjay.bpsr.resonance-stream`에 저장됩니다.
- **방화벽:** 최초 실행 시 패킷 감지를 위한 방화벽 규칙 추가 여부를 묻습니다.
- Raw Socket 사용을 위해 **관리자 권한**이 필요합니다.
