# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Code Architecture

The `flux-engine` is a Rust-based game engine built upon a workspace architecture. The core components are organized into separate crates, promoting modularity and reusability.

- **`flux_ecs`**: Implements the Entity Component System (ECS) pattern, which is central to the engine's design. It manages game entities, their components, and the systems that operate on them.

- **`flux_memory`**: Provides custom memory management utilities, including a macro for memory alignment.

- **`flux_renderer`**: Handles rendering logic, leveraging the `winit` library for windowing and event handling.

- **`main`**: The main application crate, which integrates the other components to create the final application. It initializes the `World`, adds the `RendererPlugin`, and runs the main event loop.

## Common Commands

### Building the Project

To build the entire workspace, run the following command from the root directory:

```sh
cargo build
```

To build a specific crate, use the `-p` flag:

```sh
cargo build -p flux_ecs
```

### Running Tests

To run tests for the entire workspace, use:

```sh
cargo test
```

To run tests for a specific crate:

```sh
cargo test -p flux_ecs
```

To run a single test, provide the test name as an argument:

```sh
cargo test -p flux_ecs --test my_test_name
```

### Linting the Code

To lint the code, use `clippy`:

```sh
cargo clippy
```

