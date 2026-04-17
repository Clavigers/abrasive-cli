# Abrasive CLI
Fast Rust Builds

### Cargo and Abrasive

abrasive is designed to be used as a cargo alias. None of the regular Cargo commands 
are broken by abrasive. If you run abrasive in a workspace that does not have an 
abrasive.toml file, it will just pass straight through to cargo.

The cross compilation / platform logic is the host platform is sent with every build request. If --platform flag is missing the host platform will be used as the default.

Abrasive Agent:
    the abrasive agent is a simple daemon that accepts protocol messages over a unix socket. this is an optional way to persist the websocket connection since (on my machine) that takes about 1 second to start up. That likely does not matter for a cache miss but totally kills build times for cache hits.