# iOS Development

The iOS app embeds the Rust Styrene node through UniFFI. The build creates an
XCFramework for physical iPhones and Apple Silicon or Intel simulators.

## Requirements

- Xcode with the iOS SDK
- The repository Rust toolchain
- An Apple ID configured in Xcode
- A connected iPhone with Developer Mode enabled

## Build For A Simulator

Run:

```bash
just ios-simulator
```

This command installs the Rust targets, builds the native libraries, generates
the Swift bindings, and performs an unsigned simulator build.

## Install Through Xcode

1. Connect the iPhone by USB and unlock it.
2. Run `just ios-open`.
3. Select the `StyreneMobile` target in Xcode.
4. Open **Signing & Capabilities**.
5. Select your development team.
6. Select the connected iPhone as the run destination.
7. Press **Run**.

Xcode creates a development provisioning profile automatically. App Store
credentials and distribution profiles are not required.

## Install From The Command Line

Get the device identifier:

```bash
xcrun xctrace list devices
```

Get the development team identifier from the certificate's `OU` field:

```bash
security find-certificate -c "Apple Development" -p \
  | openssl x509 -noout -subject
```

Do not use the identifier in parentheses after the certificate name. That is
the certificate owner's identifier, not the development team identifier.

Build, sign, and install the app:

```bash
just ios-install <DEVICE-ID> <TEAM-ID>
```

The device must also be available to `devicectl`. Check it with
`xcrun devicectl list devices`. If the device is unavailable, unlock it,
reconnect the cable, and confirm the trust prompt.

## Generated Files

`scripts/build-ios-ffi.sh` creates these ignored paths:

- `ios/Generated/`
- `ios/Frameworks/StyreneMobileFFI.xcframework/`
- `target/ios-*`

Run `just mobile-ios` after a Rust or UniFFI API change. The Xcode project uses
the regenerated Swift source and XCFramework directly.
