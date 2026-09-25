# signalk-kinavis

A collision watch for [Signal K](https://signalk.org) server, built on the
[KINAVIS](https://github.com/KINAVIS/kinavis) navigation crates and run as a
WASM plugin.

For every AIS target the server knows, it computes CPA and TCPA against own
vessel. For a dangerous or developing approach it adds the COLREGs ruling:
the encounter, who gives way, and what each may do. The results go back into
the server, where any Signal K app can show them:

- `navigation.closestApproach` (`distance` in metres, `timeTo` in seconds) in
  each closing target's context;
- `notifications.navigation.closestApproach.<target>` on own vessel, raised
  when a target's level changes and cleared when it no longer applies:

```text
alarm | 218784000: CPA 1.33 NM in 2:39; crossing, target on starboard, give way (Rule 15): alter to starboard, or slow down
warn  | 246754000: CPA 1.61 NM in 47:55; crossing, target on port, stand on (Rule 17): alter to starboard, or slow down, or hold course and speed
```

**An aid to the watch, not a substitute for it.** A proper lookout (COLREGs
Rule 5) and the judgement of the officer of the watch come first. The ruling
assumes every target is what AIS says it is, and a target that says nothing
is taken as power-driven; Rules 9 and 10 (narrow channels, traffic
separation schemes) are not applied.

## What it needs

- Signal K server 2.33 or later (WASM plugin support).
- Own vessel's position, course and speed from its own GNSS receiver: own
  vessel is recognised as the vessel whose position comes from a sensor
  other than AIS. Its MMSI can be set in the configuration instead.
- AIS targets.

## Configuration

| Setting | Default | |
|---|---|---|
| `cpaLimitNm` | 1.0 | a closer approach is dangerous |
| `tcpaLimitMin` | 20 | a dangerous approach sooner than this is an alarm |
| `warnWithinMin` | 60 | a dangerous approach later than the alarm but within this is a warning; beyond, nothing |
| `staleAfterS` | 180 | a report older than this is not assessed |
| `restrictedVisibility` | false | Rule 19 instead of Rules 11 to 18 |
| `assessEveryS` | 2 | assessment interval |
| `ownMmsi` | — | own vessel's MMSI, if it cannot be recognised by its GNSS receiver |

No ruling is given while own vessel is at rest (under half a knot): the
steering rules are for vessels under way.

## Build and install locally

```sh
rustup target add wasm32-wasip1
npm run build                     # cargo build --release --target wasm32-wasip1, then plugin.wasm
ln -s "$PWD" ~/.signalk/node_modules/signalk-kinavis
```

Restart the server, then enable *KINAVIS collision watch* under Server →
Plugin Config.

## Tests

```sh
npm run build && npm test
```

`npm test` checks the plugin's exports, then runs Signal K server 2.33 with
the plugin on recorded AIS traffic off Harlingen (`tests/e2e/data/`, from the
Signal K server samples) and requires the Rule 15 alarm that traffic holds. It
installs the server into a temporary directory unless `SIGNALK_SERVER` names
one, and gives the server a configuration directory of its own. CI runs the
same, and builds on the MSRV and stable.

## Design

The assessment is the `kinavis-signalk` crate: plain Rust, tested without a
server. This repository is only the host interface — the WASM exports, the
imports from the server, and the `unsafe` that crossing that boundary
requires. The KINAVIS crates themselves forbid `unsafe`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
