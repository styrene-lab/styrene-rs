{
  description = "Styrene — Rust mesh daemon, TUI, and hub services";

  nixConfig = {
    extra-substituters      = [ "https://styrene.cachix.org" ];
    extra-trusted-public-keys = [
      "styrene.cachix.org-1:oyGX4VS45l/HvLNQvBHJ+PjIQ23mUI+XTzL8aOCvXUg="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix2container = {
      url = "github:nlewo/nix2container";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, rust-overlay, nix2container }:
    let
      # NixOS module — available on all systems
      nixosModule = { config, lib, pkgs, ... }:
        with lib;
        let
          cfg = config.services.styrene-hub;
        in {
          options.services.styrene-hub = {
            enable = mkEnableOption "Styrene Hub — mesh infrastructure node";

            package = mkOption {
              type = types.package;
              default = self.packages.${pkgs.system}.styrened;
              description = "The styrened binary to use";
            };

            user = mkOption {
              type = types.str;
              default = "styrene";
              description = "User to run the hub as";
            };

            group = mkOption {
              type = types.str;
              default = "styrene";
              description = "Group to run the hub as";
            };

            dataDir = mkOption {
              type = types.path;
              default = "/var/lib/styrene-hub";
              description = "Data directory for databases, pages, and content";
            };

            role = mkOption {
              type = types.enum [ "full_node" "hub" "propagation_client" ];
              default = "hub";
              description = "Node role — hub enables propagation store";
            };

            transport = {
              enable = mkOption {
                type = types.bool;
                default = true;
                description = "Enable RNS transport backbone";
              };

              address = mkOption {
                type = types.str;
                default = "0.0.0.0:4242";
                description = "TCP transport listen address";
              };
            };

            rpc = {
              address = mkOption {
                type = types.str;
                default = "127.0.0.1:4243";
                description = "HTTP RPC listen address";
              };
            };

            pages = {
              enable = mkOption {
                type = types.bool;
                default = true;
                description = "Enable NomadNet-compatible page serving";
              };

              directory = mkOption {
                type = types.path;
                default = "/var/lib/styrene-hub/pages";
                description = "Directory containing .mu page files";
              };
            };

            openFirewall = mkOption {
              type = types.bool;
              default = false;
              description = "Open firewall for transport port";
            };

            configFile = mkOption {
              type = types.nullOr types.path;
              default = null;
              description = "Path to config.yaml (auto-generated if null)";
            };

            peers = mkOption {
              type = types.listOf (types.submodule {
                options = {
                  host = mkOption { type = types.str; };
                  port = mkOption { type = types.port; default = 4242; };
                  name = mkOption { type = types.str; default = "peer"; };
                };
              });
              default = [];
              description = "TCP client peers to connect to";
            };
          };

          config = mkIf cfg.enable {
            users.users.${cfg.user} = {
              isSystemUser = true;
              group = cfg.group;
              description = "Styrene Hub daemon user";
              home = cfg.dataDir;
              createHome = true;
            };

            users.groups.${cfg.group} = {};

            # Auto-generate config if not provided
            environment.etc."styrene/config.yaml" = mkIf (cfg.configFile == null) {
              text = let
                peerConfigs = map (p: ''
                  [[interfaces]]
                  type = "tcp_client"
                  enabled = true
                  host = "${p.host}"
                  port = ${toString p.port}
                  name = "${p.name}"
                '') cfg.peers;
              in ''
                role = "${cfg.role}"

                ${concatStringsSep "\n" peerConfigs}
              '';
            };

            systemd.services.styrene-hub = {
              description = "Styrene Hub — mesh infrastructure node";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];

              serviceConfig = {
                Type = "simple";
                User = cfg.user;
                Group = cfg.group;
                ExecStart = concatStringsSep " " ([
                  "${cfg.package}/bin/styrened"
                  "--rpc" cfg.rpc.address
                  "--db" "${cfg.dataDir}/messages.db"
                  "--config" (if cfg.configFile != null
                    then cfg.configFile
                    else "/etc/styrene/config.yaml")
                ] ++ optionals cfg.transport.enable [
                  "--transport" cfg.transport.address
                ]);
                Restart = "always";
                RestartSec = "10s";

                # Hardening
                PrivateTmp = true;
                ProtectSystem = "strict";
                ProtectHome = true;
                NoNewPrivileges = true;
                ReadWritePaths = [ cfg.dataDir "/run/styrened" ];
                StateDirectory = "styrene-hub";
                RuntimeDirectory = "styrened";
              };
            };

            # Firewall
            networking.firewall.allowedTCPPorts =
              mkIf cfg.openFirewall [ 4242 ];

            # Ensure pages directory exists
            systemd.tmpfiles.rules = mkIf cfg.pages.enable [
              "d ${cfg.pages.directory} 0755 ${cfg.user} ${cfg.group} -"
            ];
          };
        };
      rg35xxspBootChain = import ./nix/hardware/rg35xxsp/boot-chain.nix {
        pkgs = import nixpkgs { system = "aarch64-linux"; };
      };
      rg35xxspBringupImage = let
        pkgs = import nixpkgs { system = "aarch64-linux"; };
        rootSystem = nixpkgs.lib.nixosSystem {
          system = "aarch64-linux";
          modules = [
            ./nix/modules/minimal-appliance.nix
            "${nixpkgs}/nixos/modules/installer/sd-card/sd-image-aarch64.nix"
            ./nix/modules/sd-image-no-host-xattrs.nix
            ({ ... }: {
              boot.kernelPackages = pkgs.linuxPackagesFor rg35xxspBootChain.kernel;
              hardware.deviceTree.name = "allwinner/sun50i-h700-anbernic-rg35xx-sp.dtb";
              sdImage.compressImage = false;
              sdImage.firmwareSize = 256;
            })
          ];
        };
      in pkgs.callPackage ./nix/make-rg35xxsp-image.nix {
        rootFilesystemImage = rootSystem.config.sdImage.rootFilesystemImage;
        bootChain = rg35xxspBootChain.bundle;
      };
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "aarch64-linux-android" ];
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        commitSha =
          if self ? shortRev then self.shortRev
          else if self ? dirtyShortRev then self.dirtyShortRev
          else "unknown";

        # Source filtering — include Rust sources + config files
        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type)
            || builtins.match ".*\\.md$" path != null
            || builtins.match ".*\\.toml$" path != null
            || builtins.match ".*\\.json$" path != null;
        };

        commonArgs = {
          inherit src;
          pname = "styrene-rs";
          strictDeps = true;
          buildInputs = with pkgs; [
            openssl
            sqlite
            pkg-config
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
        };

        # Build deps once, reuse for all packages
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # The canonical product binary used by installation and Ghost checks
        styreneRaw = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p styrene --all-features";
        });

        # NixOS services invoke the product through a closure-complete wrapper.
        styrene = pkgs.runCommand "styrene-product" {
          nativeBuildInputs = [ pkgs.makeWrapper ];
        } ''
          mkdir -p $out/bin
          cp ${styreneRaw}/bin/styrene $out/bin/styrene-unwrapped
          makeWrapper $out/bin/styrene-unwrapped $out/bin/styrene \
            --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.sqlite pkgs.stdenv.cc.cc.lib ]}
        '';

        # The daemon binary
        styrened = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p styrened";
        });

        # The daemon with I2P proxy feature
        styrened-i2p = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p styrened --features i2p-proxy";
        });

        # The I2P proxy client binary
        styrene-i2p = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p styrene-i2p";
        });

        # The TUI binary
        styrene-tui = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p styrene-tui";
        });

        # OCI images via nix2container
        n2c = nix2container.packages.${system}.nix2container;

        oci = n2c.buildImage {
          name = "ghcr.io/styrene-lab/styrened";
          tag = commitSha;
          copyToRoot = [ styrened ];
          config = {
            entrypoint = [ "${styrened}/bin/styrened" ];
          };
        };

        oci-i2p = n2c.buildImage {
          name = "ghcr.io/styrene-lab/styrened-i2p";
          tag = commitSha;
          copyToRoot = [ styrened-i2p ];
          config = {
            entrypoint = [ "${styrened-i2p}/bin/styrened" ];
          };
        };

      in {
        packages = {
          inherit styrene styrened styrened-i2p styrene-i2p styrene-tui oci oci-i2p;
          rg35xxsp-boot-chain = rg35xxspBootChain.bundle;
          rg35xxsp-bringup-image = rg35xxspBringupImage;
          rg35xxsp-trusted-firmware-a = rg35xxspBootChain.tfA;
          rg35xxsp-u-boot = rg35xxspBootChain.uBoot;
          rg35xxsp-linux = rg35xxspBootChain.kernel;
          rg35xxsp-qemu = let
            qemuSystem = nixpkgs.lib.nixosSystem {
              inherit system;
              specialArgs = { styrenePackage = self.packages.${system}.styrene; };
              modules = [
                ./nix/modules/minimal-appliance.nix
                ./nix/modules/styrene-qemu-smoke.nix
                ({ modulesPath, styrenePackage, ... }: {
                  imports = [
                    "${modulesPath}/profiles/qemu-guest.nix"
                    "${modulesPath}/virtualisation/qemu-vm.nix"
                  ];
                  virtualisation.graphics = false;
                  services.styrene-qemu-smoke = {
                    enable = true;
                    package = styrenePackage;
                  };
                })
              ];
            };
          in qemuSystem.config.system.build.vm;
          rg35xxsp-qemu-system = let
            qemuSystem = nixpkgs.lib.nixosSystem {
              inherit system;
              specialArgs = { styrenePackage = self.packages.${system}.styrene; };
              modules = [
                ./nix/modules/minimal-appliance.nix
                ./nix/modules/styrene-qemu-smoke.nix
                ({ modulesPath, styrenePackage, ... }: {
                  imports = [
                    "${modulesPath}/profiles/qemu-guest.nix"
                    "${modulesPath}/virtualisation/qemu-vm.nix"
                  ];
                  virtualisation.graphics = false;
                  services.styrene-qemu-smoke = {
                    enable = true;
                    package = styrenePackage;
                  };
                })
              ];
            };
          in qemuSystem.config.system.build.toplevel;
          default = styrened;
        };

        checks = {
          # Run workspace tests
          workspace-tests = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
          });

          # Clippy
          workspace-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "-- -D warnings";
          });

          rg35xxsp-provenance = pkgs.runCommand "rg35xxsp-provenance-check" {
            nativeBuildInputs = [ pkgs.python3 ];
          } ''
            python3 - <<'PY'
            import pathlib, tomllib
            data = tomllib.loads(pathlib.Path("${./nix/hardware/rg35xxsp/provenance.toml}").read_text())
            assert data["schema_version"] == 1
            assert data["hardware_profile"] == "anbernic-rg35xxsp"
            names = [entry["name"] for entry in data["components"]]
            assert len(names) == len(set(names))
            assert {"u-boot", "trusted-firmware-a", "linux", "device-tree", "firmware", "nixos-userspace"} <= set(names)
            for entry in data["components"]:
                assert entry["status"] in {"unresolved", "selected", "pinned", "validated"}
                assert entry["preferred_provenance"]
                assert entry["required_evidence"]
            PY
            touch "$out"
          '';
        };

        devShells.default = craneLib.devShell {
          packages = with pkgs; [
            rust-analyzer
            cargo-watch
            cargo-edit
            cargo-ndk
          ];
        };
      }
    ) // {
      # NixOS module (system-independent)
      nixosModules.default = nixosModule;
      nixosModules.rg35xxsp-hardware = import ./nix/hardware/rg35xxsp;
      nixosModules.rpi4b-hardware = import ./nix/hardware/rpi4b;

      nixosConfigurations.rpi4-builder = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = [
          ./nix/hardware/rpi4b
          ./nix/hosts/rpi4-builder.nix
          ({ ... }: { system.stateVersion = "26.05"; })
        ];
      };

      nixosConfigurations.rpi4-appliance = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        specialArgs = { styrenePackage = self.packages.aarch64-linux.styrene; };
        modules = [
          ./nix/hardware/rpi4b
          ./nix/modules/minimal-appliance.nix
          ./nix/modules/styrene-qemu-smoke.nix
          ./nix/hosts/rpi4-appliance.nix
        ];
      };

      nixosConfigurations.rg35xxsp-qemu = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        specialArgs = { styrenePackage = self.packages.aarch64-linux.styrene; };
        modules = [
          ./nix/modules/minimal-appliance.nix
          ./nix/modules/styrene-qemu-smoke.nix
          ({ modulesPath, styrenePackage, ... }: {
            imports = [
              "${modulesPath}/profiles/qemu-guest.nix"
              "${modulesPath}/virtualisation/qemu-vm.nix"
            ];
            virtualisation.graphics = false;
            services.styrene-qemu-smoke = {
              enable = true;
              package = styrenePackage;
            };
          })
        ];
      };
    };
}
