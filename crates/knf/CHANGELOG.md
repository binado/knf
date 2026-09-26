# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.3](https://github.com/binado/knf/compare/knf-cli-v0.3.2...knf-cli-v0.3.3) - 2026-09-26

### Added

- add cascading configuration file discovery ([#34](https://github.com/binado/knf/pull/34))

### Other

- parse --accumulate as a typed target ([#37](https://github.com/binado/knf/pull/37))
- rename Python API to load ([#35](https://github.com/binado/knf/pull/35))

## [0.3.2](https://github.com/binado/knf/compare/knf-cli-v0.3.1...knf-cli-v0.3.2) - 2026-09-25

### Added

- expose opt-in interpolation in pyknf ([#32](https://github.com/binado/knf/pull/32))

## [0.3.1](https://github.com/binado/knf/compare/knf-cli-v0.3.0...knf-cli-v0.3.1) - 2026-09-25

### Other

- publish only the pyknf wheel ([#30](https://github.com/binado/knf/pull/30))

## [0.3.0](https://github.com/binado/knf/compare/knf-cli-v0.2.0...knf-cli-v0.3.0) - 2026-09-25

### Added

- [**breaking**] replace per-path merge rules with --shallow ([#22](https://github.com/binado/knf/pull/22))

### Other

- [**breaking**] merge knf-interp and knf-config into knf-core ([#25](https://github.com/binado/knf/pull/25))

## [0.2.0](https://github.com/binado/knf/compare/knf-cli-v0.1.3...knf-cli-v0.2.0) - 2026-08-28

### Other

- [**breaking**] split the pipeline into knf-config and fold knf-dotted away ([#19](https://github.com/binado/knf/pull/19))

## [0.1.3](https://github.com/binado/knf/compare/knf-cli-v0.1.2...knf-cli-v0.1.3) - 2026-08-24

### Added

- --interpolate, variable and environment references ([#15](https://github.com/binado/knf/pull/15))

## [0.1.2](https://github.com/binado/knf/compare/knf-cli-v0.1.1...knf-cli-v0.1.2) - 2026-08-16

### Added

- group merge rules in --help and rename --null-placeholder to --null-as ([#13](https://github.com/binado/knf/pull/13))

## [0.1.1](https://github.com/binado/knf/compare/knf-cli-v0.1.0...knf-cli-v0.1.1) - 2026-08-16

### Added

- --null-placeholder, and drop the null-in-TOML origin trace ([#12](https://github.com/binado/knf/pull/12))
- per-path merge strategies (--append, --replace, --fail) ([#10](https://github.com/binado/knf/pull/10))
