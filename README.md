# Nest STM bus app

**Montreal STM bus departures on a Nest Learning Thermostat (2nd
generation) running Nest Assistant.** Its card shows up on the Nest's
Devices carousel; open it and the next departures for your stop are on the
round screen — bus number, destination, and minutes away — three rows at a
time, the ring turning the pages.

## Install

On the Home Assistant that talks to your Nest, open the Nest Assistant
integration → **Configure** → **Apps**: repositories tagged `nest-assistant`
appear there; install this one. Then configure the stop in **Configure** →
**STM bus**: the **stop id** (the STM's numeric stop code, e.g. `55404`) and,
if you have one, an **API key** from the STM's developer portal
(portail.developpeurs.stm.info). Home Assistant polls the STM's real-time
feed for that stop and pushes the departures to this app; the Nest never
talks to the internet itself. Without a stop configured the app shows its
waiting screen.

## What an app is

An app never touches the Nest's screen, fonts, beeper or input. It is a
statically linked ARM binary that the Nest starts when you open its card and
stops when you leave, confined to its own directory and its own user. It
speaks newline-delimited JSON on file descriptor 3: the Nest sends it
`open`, `wheel`, `data` and `stop`; it answers with scenes (fill and stroke
paths, text runs, icons — all validated and rendered by the Nest), `tick`,
`event` and `close`. This app is read-only: it draws whatever departures
arrive, tolerates any malformed payload, and never panics. The protocol is
documented in the SDK's doc comments in [`sdk/`](sdk/) — the same SDK the
[demo app](https://github.com/elecnix/nest-demo-app) exercises end to end.

## Build and publish

```sh
./build.sh            # packages stm/dist/stm/nest-app.tar.gz for the Nest
cargo test            # the SDK and app unit tests
```

`build.sh` cross-compiles for `armv7-unknown-linux-musleabihf` (the toolchain
and target pin are in `rust-toolchain.toml`), copies the binary as `app`, and
packs exactly `manifest.json` + `app` into the gzipped tar a release must
carry. Publishing is a release with that tar as its `nest-app.tar.gz` asset,
on a repository carrying the `nest-assistant` topic — this one.

## License

MIT — see [LICENSE](LICENSE). The bus icon it draws by name is rendered by
the Nest itself from its own icon table. This project is not affiliated with
the Société de transport de Montréal; departure data comes from the STM's
public real-time API under its developer terms.