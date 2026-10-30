# Sticks Readme

> *Put some sticks in your HTML.*

**Website:** [https://sticksproject.org/](https://sticksproject.org/)

Sticks is an opinionated, zero-build, server-rendered web platform packaged as a single static Rust binary. It offers the drop-in file-routing simplicity of classic PHP combined with modern type safety, structural injection immunity, and native response streaming.

Templates are standard `.html` files enhanced with a minimal delimiter model: `||` opens a block and `|` closes it.

---

## Repository Structure

This repository contains both the Sticks runtime engine and an example task list application:

```text
.
├── sticks/          # The Sticks engine & runtime (Rust binary)
│   ├── Cargo.toml
│   └── src/
└── sticksdemo/      # Full-stack task management demo application
    ├── sticks.toml  # Site configuration
    ├── data/        # Database schemas and local SQLite storage
    └── public/      # Routed HTML pages and static assets
```

---

## Key Concepts

* **The Two Zones:** Execution flows strictly top-to-bottom:
  * **Effect Zone:** Code before the first visible byte of HTML. Handles state, mutations (`db.exec`), and `on POST` / `on GET` handlers. Calling `redirect()` halts execution immediately before markup streams.
  * **View Zone:** Begins with the first output byte. Commits headers, locks writes, and streams output chunk-by-chunk. Side effects are strictly forbidden here.
* **Rule S4 (No Block Nesting):** Blocks cannot be nested inside other blocks. Deep UI branching is solved via the sibling block pattern, component composition, or client-side CSS state projection.
* **Structural Security:** SQL queries must be string literals with positional `?` parameters (preventing SQL injection). Output is escaped by default according to type. Template re-scanning is impossible.
* **Fail at Boot:** All templates are scanned, parsed, and type-checked before the server starts handling traffic.

---

## Quickstart

### 1. Prerequisites
* [Rust & Cargo](https://rustup.rs/) (edition 2021+)

### 2. Build the Server
From the repository root:

```bash
cargo build --release --manifest-path sticks/Cargo.toml
```

The compiled binary will be located at `sticks/target/release/sticks`.

### 3. Run the Demo
You can run the demo directly from the repository root:

```bash
./sticks/target/release/sticks serve sticksdemo/sticks.toml
```

*(Alternatively, run in development mode with `cargo run`):*
```bash
cargo run --manifest-path sticks/Cargo.toml -- serve sticksdemo/sticks.toml
```

### 4. Open in Browser
Visit [http://localhost:3030](http://localhost:3030).

---

## Project Status

* **Language Specification:** `v0.3.0-draft` (design frozen for 1.0 scope).
* **Engine Implementation:** `v1.0.0-draft` (functional prototype / proof-of-concept).

