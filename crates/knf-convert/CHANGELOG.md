# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0](https://github.com/binado/knf/compare/knf-convert-v0.9.0...knf-convert-v0.10.0) - 2026-11-26

### Fixed

- [**breaking**] name the failing input, key and file in diagnostics ([#98](https://github.com/binado/knf/pull/98))

### Other

- *(convert)* [**breaking**] make option combinations unrepresentable ([#99](https://github.com/binado/knf/pull/99))
- *(interp)* [**breaking**] parse strings once and carry failures in the caches ([#97](https://github.com/binado/knf/pull/97))
- [**breaking**] rename layers to configs ([#94](https://github.com/binado/knf/pull/94))

## [0.9.0](https://github.com/binado/knf/compare/knf-convert-v0.8.1...knf-convert-v0.9.0) - 2026-11-21

### Added

- [**breaking**] unify configuration layers on knf::Value ([#82](https://github.com/binado/knf/pull/82))

## [0.8.1](https://github.com/binado/knf/compare/knf-convert-v0.8.0...knf-convert-v0.8.1) - 2026-11-20

### Fixed

- support YAML output in the Python API ([#79](https://github.com/binado/knf/pull/79))

## [0.8.0](https://github.com/binado/knf/compare/knf-convert-v0.7.1...knf-convert-v0.8.0) - 2026-11-20

### Added

- *(cli)* add YAML output ([#78](https://github.com/binado/knf/pull/78))
- run commands with resolved config and add conversion and Python helpers ([#69](https://github.com/binado/knf/pull/69))

### Other

- split AGENTS.md and clean up invariant-enforcing comments ([#72](https://github.com/binado/knf/pull/72))
