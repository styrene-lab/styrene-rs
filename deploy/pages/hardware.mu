`F58a`cHardware Guide`f

`[< Back to Index`:/page/index.mu]

-

>Supported Hardware

Styrene runs on anything that runs Linux or macOS.
The transport layer determines what physical links
are available.

-

>Transport: TCP/IP (Internet)

Any device with a network connection.
This is how most users connect to the community hub.

`=
# Config
[[interfaces]]
type = "tcp_client"
host = "rns.styrene.io"
port = 4242
`=

-

>Transport: LoRa Radio (RNode)

Long-range, low-bandwidth mesh over LoRa.
Requires an RNode device (ESP32 + SX127x/SX126x).

Typical range: 2-15km line-of-sight.
Bandwidth: 1-20 kbps depending on settings.

Build or buy an RNode:
https://unsigned.io/rnode/

-

>Transport: WiFi Mesh (802.11s / BATMAN-adv)

Local-area mesh over WiFi. No access point needed.
Devices form an ad-hoc mesh network automatically.

Best for: building-scale or campus-scale deployments.

-

>Edge Devices

>>Raspberry Pi Zero 2W
Smallest supported device. Runs styrened with LoRa
transport via RNode hat. ~$15 + RNode hardware.

>>Raspberry Pi 4/5
Full hub capability. Can run transport, propagation,
page hosting, and fleet management simultaneously.

>>Any x86_64 Linux Server
The community hub at rns.styrene.io runs on a k3s
cluster (containerized styrened).

-

>Device Provisioning

Styrene Edge provides NixOS configurations for
mesh nodes. Write an image, plug it in, it joins
the mesh on first boot.

`=
# Generate edge image (requires nex)
nex edge build --profile mesh-node --target rpi4
`=

-

`[< Back to Index`:/page/index.mu]
