# Third-party notices

This repository is licensed under the [MIT License](LICENSE). `zte-esim`, the agent's on-demand
eSIM helper, also contains the components below. This file accompanies every release that ships
`zte-esim`. `zte-agent` itself links none of them.

## euicc (LGPL-2.1-only)

The `euicc/` directory is a Rust translation of libeuicc, the eSIM library of
[lpac](https://github.com/estkme-group/lpac) (commit `c7c8d4f`), vendored from the OpenU60 fork's
translation. It is linked into `zte-esim`.

```
Copyright (C) 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
Copyright (C) 2026 Jesther Silvestre (Rust translation)
```

The euicc library and its use are covered by the **GNU Lesser General Public License, version
2.1 only**. A copy of the license is in [`euicc/LICENSE`](euicc/LICENSE) and is attached to each
release as `LICENSE-LGPL-2.1.txt`. Each file in `euicc/src/` states which libeuicc file it was
translated from and how it was changed.

**Source code and relinking.** The complete corresponding source of the euicc library and of
`zte-esim` for a release is this repository at that release's tag (also the "Source code" archives
on the release page): https://github.com/dklasens/MU5250-OpenUI. To use a modified euicc library,
edit `euicc/` and rebuild the helper, then deploy it with `./deploy.sh`:

```sh
cargo build --release --locked --target aarch64-unknown-linux-musl -p zte-esim
```

You may modify `zte-esim` for your own use and reverse engineer it to debug such modifications.

### cJSON (MIT), inside euicc

`euicc/src/es9p.rs` reads and writes JSON the way libeuicc's cJSON did; that part follows cJSON 1.7.18 (as pinned by lpac) and carries its notice:

```
Copyright (c) 2009-2017 Dave Gamble and cJSON contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## QMI-over-QRTR transport (MIT)

`esim-helper/src/qrtr_uim.rs` is based on lpac's QMI-over-QRTR APDU driver
(`driver/apdu/qmi_qrtr.c`, `qmi_common.c`), by way of the OpenU60 fork.

```
Copyright (c) 2024, Luca Weiss <luca.weiss@fairphone.com>
Copyright (c) 2026, Jesther Silvestre

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## GSMA RSP2 Root CI1 certificate

`esim-helper/certs/gsma-rsp2-root-ci1.pem` is the public root certificate of the GSMA eSIM
certificate issuer (SHA-256 fingerprint `5E:3E:91:FD:…:80:A5:6B:B3`, valid 2017-02-22 to
2052-02-21). It is embedded in `zte-esim` so that operator servers' TLS certificates can be
verified; nothing else is trusted for those connections.

## Rust crates

`zte-esim` links these crates from crates.io (for the router target), each under the license
shown. Each crate's license text and copyright notices are in its published source.

| Crate | Version | License |
|---|---|---|
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| cfg-if | 1.0.4 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| generic-array | 0.14.7 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| itoa | 1.0.17 | MIT OR Apache-2.0 |
| libc | 0.2.182 | MIT OR Apache-2.0 |
| memchr | 2.8.0 | Unlicense OR MIT |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rustls | 0.23.43 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-webpki | 0.103.14 | ISC |
| serde_core | 1.0.228 | MIT OR Apache-2.0 |
| serde_json | 1.0.149 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| subtle | 2.6.1 | BSD-3-Clause |
| typenum | 1.19.0 | MIT OR Apache-2.0 |
| untrusted | 0.9.0 | ISC |
| zeroize | 1.9.0 | Apache-2.0 OR MIT |
| zmij | 1.0.21 | MIT |

## Dashboard: jsQR (Apache-2.0)

The dashboard's SIM Management "Upload QR image" step decodes the picture in the browser with
[jsQR](https://github.com/cozmo/jsQR) 1.4.0 when the browser has no built-in QR detector. It is
loaded only on that step (a separate `jsqr` chunk). jsQR is licensed under the Apache License,
Version 2.0; its license text is in the package (`node_modules/jsqr/LICENSE`) and at
https://www.apache.org/licenses/LICENSE-2.0.
