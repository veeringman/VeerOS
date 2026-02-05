<!-- LOGO PLACEHOLDER -->
![VeerOS Logo](./docs/veeros.logo.png)

**VeerOS** is a lightweight, secure, and experimental operating system designed for embedded devices (ESP32 RISC-V).  
Built in Rust, VeerOS explores modern OS concepts — secure boot, OTA updates, and remote management — while staying minimal and educational.

---

## ✨ Features
- Secure Boot  
- OTA Updates  
- UEFI-like Bootloader
- Process Model and Threads
- Networking Stack
- Virtual Memory
- SSH Access
- Rust-based Kernel  

---

## 🚀 Roadmap
- [x] Bootloader with menu  
- [x] Secure boot verification  
- [x] OTA slot + fallback kernel  
- [x] TCP/IP stack  
- [x] SSH daemon  
- [x] Filesystem + shell  

---

## 📂 Repository Structure

```text
/bootloader   -> Stage 1 loader
/kernel       -> Core OS kernel
/net          -> Networking stack
/shell        -> Command interpreter
/docs         -> Documentation
```

## 📐 Architecture

VeerOS uses a clean, portable, multi-architecture kernel design.

👉 **Read the full architecture overview:**  
[VeerOS System Architecture](/docs/architecture.md)

