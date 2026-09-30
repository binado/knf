# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0](https://github.com/binado/knf/compare/knf-core-v0.6.0...knf-core-v0.7.0) - 2026-09-30

### Added

- *(interp)* [**breaking**] interpolate reference bodies ([#61](https://github.com/binado/knf/pull/61))

## [0.6.0](https://github.com/binado/knf/compare/knf-core-v0.5.1...knf-core-v0.6.0) - 2026-09-30

### Added

- [**breaking**] remove --strict and make native merge infallible ([#58](https://github.com/binado/knf/pull/58))
- [**breaking**] merge interpolation references as the values they name ([#57](https://github.com/binado/knf/pull/57))
- add configurable object inheritance ([#55](https://github.com/binado/knf/pull/55))

### Other

- drop the error parameter from merge_fields ([#59](https://github.com/binado/knf/pull/59))
- [**breaking**] drop MergeOptions::LAST_WINS ([#53](https://github.com/binado/knf/pull/53))

## [0.5.1](https://github.com/binado/knf/compare/knf-core-v0.5.0...knf-core-v0.5.1) - 2026-09-29

### Added

- add interpolation context sources ([#51](https://github.com/binado/knf/pull/51))

## [0.5.0](https://github.com/binado/knf/compare/knf-core-v0.4.0...knf-core-v0.5.0) - 2026-09-27

### Added

- *(cli)* replace --set with -c and add -i alias ([#49](https://github.com/binado/knf/pull/49))
- [**breaking**] select shallow replacements with quoted path globs ([#46](https://github.com/binado/knf/pull/46))

### Other

- condense README, --help text and code comments ([#50](https://github.com/binado/knf/pull/50))

## [0.4.0](https://github.com/binado/knf/compare/knf-core-v0.3.3...knf-core-v0.4.0) - 2026-09-27

### Added

- [**breaking**] keep JSON and TOML pipelines native ([#45](https://github.com/binado/knf/pull/45))
- [**breaking**] accept key paths in --shallow ([#43](https://github.com/binado/knf/pull/43))

### Migration

| Previous behavior/API | Replacement |
| --- | --- |
| Mixed JSON/TOML layers | Merge one format per invocation or Python `load()` call |
| `--input-format FORMAT` | `-f/--format FORMAT` |
| `-f` selecting a different output encoding | `-f` selects the entire native pipeline; use a separate conversion tool if needed |
| `--null-as` | Removed; TOML `-c proxy=null` now produces the string `"null"` |
| `knf::Value`, `Map`, `Number`, conversion helpers/errors | Native JSON/TOML values and `ConfigValue`/`ConfigObject` traits |
| `load_layers` returning values and format lists | `Layers::Json` or `Layers::Toml` |
| `EnvValue { raw, typed }` | `Env::lookup` returns raw `Option<String>` |
| `format::parse(format, …)` / `emit(value, format, …)` | Generic `parse::<V>(text, source)` / `emit(value, pretty)` |
| Python TOML datetime strings | Native Python date/time objects; fractional seconds truncate to microseconds |
| `--shallow` (bare) / `--shallow=foo` | `--shallow '*'` / `--shallow 'foo.*'` |

## [0.3.3](https://github.com/binado/knf/compare/knf-core-v0.3.2...knf-core-v0.3.3) - 2026-09-27

### Added

- expose shared file discovery and filtering APIs ([#42](https://github.com/binado/knf/pull/42))

## [0.3.0](https://github.com/binado/knf/compare/knf-core-v0.2.0...knf-core-v0.3.0) - 2026-09-25

### Added

- [**breaking**] replace per-path merge rules with --shallow ([#22](https://github.com/binado/knf/pull/22))

### Other

- [**breaking**] merge knf-interp and knf-config into knf-core ([#25](https://github.com/binado/knf/pull/25))

## [0.2.0](https://github.com/binado/knf/compare/knf-core-v0.1.3...knf-core-v0.2.0) - 2026-08-28

### Other

- [**breaking**] split the pipeline into knf-config and fold knf-dotted away ([#19](https://github.com/binado/knf/pull/19))

## [0.1.3](https://github.com/binado/knf/compare/knf-core-v0.1.2...knf-core-v0.1.3) - 2026-08-24

### Added

- --interpolate, variable and environment references ([#15](https://github.com/binado/knf/pull/15))

## [0.1.1](https://github.com/binado/knf/compare/knf-core-v0.1.0...knf-core-v0.1.1) - 2026-08-16

### Added

- per-path merge strategies (--append, --replace, --fail) ([#10](https://github.com/binado/knf/pull/10))
