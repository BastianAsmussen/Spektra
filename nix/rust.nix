{ inputs, ... }:
{
  systems = [ "x86_64-linux" ];

  perSystem =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      craneLib = inputs.crane.mkLib pkgs;
      src = lib.cleanSourceWith {
        src = ../.;
        name = "spektra-source";
        filter =
          path: type:
          (craneLib.filterCargoSources path type) || (builtins.match ".*\\.(proto|sql|html)$" path != null);
      };

      swaggerUi = pkgs.fetchurl {
        url = "https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.17.14.zip";
        hash = "sha256-SBJE0IEgl7Efuu73n3HZQrFxYX+cn5UU5jrL4T5xzNw=";
      };

      commonArgs = {
        inherit src;

        strictDeps = true;

        buildInputs = with pkgs; [
          postgresql_18.lib
          soapysdr
        ];
        nativeBuildInputs = with pkgs; [
          protobuf
          pkg-config
          rustPlatform.bindgenHook
        ];

        preBuild = ''
          install -Dm644 ${swaggerUi} "$NIX_BUILD_TOP/swagger-ui.zip"

          export SWAGGER_UI_DOWNLOAD_URL="file://$NIX_BUILD_TOP/swagger-ui.zip"
        '';
      };

      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      postgresFixture = ''
        export PGDATA="$TMPDIR/pgdata"
        export PGHOST=127.0.0.1
        export PGUSER=spektra_test

        initdb --username=spektra_test --auth=trust --no-locale --encoding=UTF8 >/dev/null
        pg_ctl start -w -o "-c listen_addresses=127.0.0.1 -c port=5432 -k $TMPDIR" >/dev/null
        createdb --username=spektra_test --host=127.0.0.1 spektra_test

        export TEST_DATABASE_URL="postgres://spektra_test:test_password@127.0.0.1:5432/spektra_test"
      '';

      crate =
        {
          name,
          description,
          extraArgs ? { },
        }:
        craneLib.buildPackage (
          commonArgs
          // extraArgs
          // {
            inherit cargoArtifacts;

            pname = name;
            cargoExtraArgs = "--locked --package ${name}";

            doCheck = false;
            meta = {
              inherit description;

              license = lib.licenses.mit;
              maintainers = [ lib.maintainers.BastianAsmussen ];
              platforms = lib.platforms.linux;
              mainProgram = name;
            };
          }
        );
      styleSource = lib.fileset.toSource {
        root = ../server;
        fileset = lib.fileset.unions [
          ../server/assets
          ../server/templates
        ];
      };

      crossPkgs = pkgs.pkgsCross.aarch64-multiplatform;
      craneCross = inputs.crane.mkLib crossPkgs;

      crossArgs = {
        inherit src;

        strictDeps = true;
        pname = "node-agent";
        cargoExtraArgs = "--locked --package node-agent";

        buildInputs = [ crossPkgs.soapysdr ];
        nativeBuildInputs = with crossPkgs; [
          protobuf
          pkg-config
          rustPlatform.bindgenHook
        ];

        depsBuildBuild = [ crossPkgs.stdenv.cc ];

        CARGO_BUILD_TARGET = "aarch64-unknown-linux-gnu";
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${crossPkgs.stdenv.cc.targetPrefix}cc";
      };
    in
    {
      formatter = pkgs.nixfmt-tree;
      packages = {
        static =
          pkgs.runCommand "spektra-static"
            {
              nativeBuildInputs = with pkgs; [
                tailwindcss_4
                brotli
                gzip
              ];

              meta = {
                description = "Spektra web client assets: stylesheet, Leaflet, HTMX.";
                license = lib.licenses.mit;
                platforms = lib.platforms.all;
              };
            }
            ''
              mkdir -p "$out"
              tailwindcss -i ${styleSource}/assets/app.css -o "$out/app.css" --minify
              install -Dm444 ${styleSource}/assets/*.js "$out/"
              install -Dm444 ${styleSource}/assets/vendor/* "$out/"

              for f in "$out"/*; do
                brotli -q 11 -k "$f"
                gzip -9 -k "$f"
              done
            '';

        server = crate {
          name = "server";
          description = "Spektra central server: gRPC ingest, deviation detection, web API.";
        };

        node-agent = crate {
          name = "node-agent";
          description = "Spektra node agent: SDR sampling, local DSP, aggregation and reporting.";
        };

        node-agent-aarch64 = craneCross.buildPackage (
          crossArgs
          // {
            cargoArtifacts = craneCross.buildDepsOnly crossArgs;

            doCheck = false;
            meta = {
              description = "Spektra node agent, cross-built for the aarch64 radio node.";
              license = lib.licenses.mit;
              maintainers = [ lib.maintainers.BastianAsmussen ];
              platforms = [ "aarch64-linux" ];
              mainProgram = "node-agent";
            };
          }
        );
      };

      checks = {
        inherit (config.packages) server node-agent static;

        clippy = craneLib.cargoClippy (
          commonArgs
          // {
            inherit cargoArtifacts;

            cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings -A clippy::multiple_crate_versions";
          }
        );

        fmt = craneLib.cargoFmt { inherit src; };
        tests = craneLib.cargoNextest (
          commonArgs
          // {
            inherit cargoArtifacts;

            cargoNextestExtraArgs = "--workspace";
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.postgresql_18 ];
            preCheck = postgresFixture;

            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          }
        );
      };

      devShells.default = craneLib.devShell {
        inputsFrom = [ cargoArtifacts ];
        packages = with pkgs; [
          brotli
          cargo-bloat
          cargo-nextest
          gzip
          rust-analyzer
          diesel-cli
          protobuf
          pkg-config
          postgresql_18.lib
          rustPlatform.bindgenHook
          tailwindcss_4
          (writeShellScriptBin "spektra-static" ''
            set -euo pipefail

            root="$(git rev-parse --show-toplevel)"
            out="$root/server/assets/dist"

            mkdir -p "$out"
            cp -f "$root"/server/assets/vendor/* "$root"/server/assets/*.js "$out/"

            watch=0
            for arg in "$@"; do
              if [ "$arg" = "--watch" ]; then watch=1; fi
            done

            ${lib.getExe tailwindcss_4} -i "$root/server/assets/app.css" -o "$out/app.css"
            for f in "$out"/*; do
              brotli -q 11 -k "$f"
              gzip -9 -k "$f"
            done

            if [ "$watch" = 1 ]; then
              exec ${lib.getExe tailwindcss_4} \
                -i "$root/server/assets/app.css" -o "$out/app.css" --watch
            fi
          '')
          soapysdr-with-plugins
          rtl-sdr
          airspy
        ];

        SOAPY_SDR_PLUGIN_PATH = "${pkgs.soapysdr-with-plugins}/${pkgs.soapysdr-with-plugins.searchPath}";
        RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
      };
    };
}
