{
  description = "Nixly Media - desktop client for nixly media server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rust = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "x86_64-pc-windows-gnu" ];
        };

        runtimeLibs = with pkgs; [
          mpv-unwrapped
          libGL
          libva
          libvdpau
          libxkbcommon
          wayland
          fontconfig
          freetype
          vulkan-loader
          alsa-lib
          udev
        ];

        buildInputs = with pkgs; [
          mpv-unwrapped
          openssl
          fontconfig
          freetype
          libxkbcommon
          wayland
          libGL
          udev
        ];

        nativeBuildInputs = with pkgs; [
          pkg-config
          rust
          cmake
        ];

        nixlymedia = pkgs.rustPlatform.buildRustPackage {
          pname = "nixlymedia";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          inherit nativeBuildInputs buildInputs;

          postFixup = ''
            patchelf \
              --set-rpath "${pkgs.lib.makeLibraryPath runtimeLibs}" \
              $out/bin/nixlymedia
          '';
        };
      in {
        packages.default = nixlymedia;
        packages.nixlymedia = nixlymedia;

        apps.default = {
          type = "app";
          program = "${nixlymedia}/bin/nixlymedia";
        };

        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs;
          buildInputs = buildInputs ++ (with pkgs; [
            pkgsCross.mingwW64.stdenv.cc
            pkgsCross.mingwW64.windows.pthreads
          ]);
          LD_LIBRARY_PATH = "/run/opengl-driver/lib:/usr/lib:"
            + pkgs.lib.makeLibraryPath runtimeLibs;
          shellHook = ''
            export RUST_SRC_PATH="${rust}/lib/rustlib/src/rust/library"
            export LIBVA_DRIVERS_PATH=''${LIBVA_DRIVERS_PATH:-/run/opengl-driver/lib/dri:/usr/lib/dri}
            export VDPAU_DRIVER_PATH=''${VDPAU_DRIVER_PATH:-/run/opengl-driver/lib/vdpau:/usr/lib/vdpau}
            has_nvidia=0; has_amd=0; has_intel=0
            for v in /sys/class/drm/card[0-9]/device/vendor; do
              vid=$(cat "$v" 2>/dev/null)
              case "$vid" in
                0x10de) has_nvidia=1 ;;
                0x1002) has_amd=1 ;;
                0x8086) has_intel=1 ;;
              esac
            done
            if [ -z "$LIBVA_DRIVER_NAME" ]; then
              if [ "$has_nvidia" = "1" ] && [ -e /run/opengl-driver/lib/dri/nvidia_drv_video.so ]; then
                export LIBVA_DRIVER_NAME=nvidia
                export NVD_BACKEND=''${NVD_BACKEND:-direct}
              elif [ "$has_amd" = "1" ] && [ -e /run/opengl-driver/lib/dri/radeonsi_drv_video.so ]; then
                export LIBVA_DRIVER_NAME=radeonsi
              elif [ "$has_intel" = "1" ] && [ -e /run/opengl-driver/lib/dri/iHD_drv_video.so ]; then
                export LIBVA_DRIVER_NAME=iHD
              elif [ "$has_intel" = "1" ] && [ -e /run/opengl-driver/lib/dri/i965_drv_video.so ]; then
                export LIBVA_DRIVER_NAME=i965
              fi
            fi
            if [ "$NIXLY_FORCE_NVIDIA" = "1" ] && [ "$has_nvidia" = "1" ] && [ -e /run/opengl-driver/share/glvnd/egl_vendor.d/10_nvidia.json ]; then
              export __NV_PRIME_RENDER_OFFLOAD=''${__NV_PRIME_RENDER_OFFLOAD:-1}
              export __EGL_VENDOR_LIBRARY_FILENAMES=''${__EGL_VENDOR_LIBRARY_FILENAMES:-/run/opengl-driver/share/glvnd/egl_vendor.d/10_nvidia.json}
            fi
            echo "nixlymedia dev shell (LIBVA_DRIVER_NAME=$LIBVA_DRIVER_NAME, GPU=$(if [ -n "$__EGL_VENDOR_LIBRARY_FILENAMES" ]; then echo nvidia; else echo default; fi)). cargo run = test app. NIXLY_FORCE_NVIDIA=1 to try NVIDIA EGL (may break Wayland)."
          '';
        };
      });
}
