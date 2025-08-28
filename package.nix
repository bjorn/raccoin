{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  makeWrapper,
  libxkbcommon,
  wayland,
  cairo,
  pango,
  harfbuzz,
  fontconfig,
  freetype,
  alsa-lib,
  libGL,
  xorg,
  openssl,
  nix-update-script,
  withWayland ? true,
  withX11 ? true,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "raccoin";
  version = "0.2.0";

  src = lib.cleanSourceWith {
    filter =
      name: type:
      !(
        type == "directory"
        && builtins.elem (baseNameOf name) [
          ".github"
          "target"
        ]
      );
    src = lib.cleanSource ./.;
  };
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];
  buildInputs = [
    cairo
    pango
    harfbuzz
    fontconfig
    freetype
    alsa-lib
    libGL
    openssl
    libxkbcommon
  ]
  ++ lib.optionals withWayland [
    wayland
  ]
  ++ lib.optionals withX11 [
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];

  # deactivated tests by cargo as build sandbox does not have the necessary display
  # also no tests available yet – if true, checks may fail simply due to missing tests
  doCheck = false;
  # alternatively in case tests are added later on
  #doCheck = true;
  #checkPhase = ''
  #  cargo test --lib -- --nocapture
  #'';
  #passthru.tests.version = testers.testVersion { package = raccoin; };

  postFixup = ''
    wrapProgram $out/bin/raccoin \
      --set-default SLINT_BACKEND winit \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ libxkbcommon xorg.libX11 fontconfig ]}
  '';

  passthru.updateScript = nix-update-script { };

  meta = {
    description = "Crypto portfolio & capital-gains reporting tool (Rust + Slint)";
    longDescription = ''
      This GUI Tool enables accounting with the quite basic principle
      "first in, first out" (FIFO) in multiple wallets. It is for
      e.g. bitcoiners that actually use the currency and want
      reports on their holdings.
    '';
    homepage = "https://raccoin.org";
    changelog = "https://github.com/bjorn/raccoin/releases/tag/v${finalAttrs.version}";
    license = lib.licenses.gpl3Plus;
    mainProgram = "raccoin";
    platforms = lib.platforms.linux;
    maintainers = with lib.maintainers; [ vv01f ];
  };
})
