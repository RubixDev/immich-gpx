# Immich GPX

Add location data from recorded GPX tracks to assets on Immich.

## Installation

```bash
cargo install --git https://github.com/RubixDev/immich-gpx
```

## Usage

First, set the `IMMICH_API_KEY` environment variable to an Immich API key with the `asset.read` and `asset.update` permissions.
You can also do this in a `.env` file.
Then list the available options with:

```bash
immich-gpx --help
```

A basic usage example may look like this:

```bash
immich-gpx --server https://immich.example.com --owner <my user id> gpx/*
```
