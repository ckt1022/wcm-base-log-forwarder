# wcm-base-log-forwarder

A simple log forwarder tool built with Rust, designed to process and forward streamed logs efficiently.

---

## 🚀 Getting Started

### 1. Build the Project

```bash
cargo build
```

---

### 2. Run with Log Generator

Use the following command to pipe logs into the forwarder:

```bash
go run ../abc.go -rate 5000 -duration 120 | target/debug/wcm-base-log-forwarder
```

---

## ⚙️ Parameters

### Log Generator (`abc.go`)

* `-rate` : Number of logs generated per second
* `-duration` : Duration of log generation (in seconds)

Example:

```bash
go run ../abc.go -rate 5000 -duration 120
```

---

## 📌 Requirements

* Rust (https://www.rust-lang.org/)
* Go (https://golang.org/)

---

## 📂 Project Structure

```
.
├── src/
├── target/
├── README.md
```

---

## 🧪 Use Cases

This tool is suitable for:

* High-throughput log testing
* Pipeline validation
* Streaming performance benchmarking

---

## ⚠️ Notes

* The compiled binary will be located at:

  ```
  target/debug/wasm-base-log-forwarder
  ```
* Adjust `rate` and `duration` based on your testing requirements.
* Make sure the `abc.go` file exists in the specified relative path.

---
