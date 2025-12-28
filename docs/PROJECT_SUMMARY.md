# FIPA WASM Distributed Agent System

## Project Overview

The FIPA WASM Agent System is a modern, open-source implementation of FIPA (Foundation for Intelligent Physical Agents) specifications built in Rust. It combines the portability of WebAssembly (WASM) agents with a distributed architecture powered by libp2p networking and Raft consensus, enabling mobile agents that can migrate between nodes while maintaining strong consistency guarantees.

The project draws significant inspiration from JADE (Java Agent Development Framework), adapting its proven architectural patterns and programming model for a contemporary Rust/WASM environment.

## Key Features

**FIPA Protocol Compliance**
- Full ACL (Agent Communication Language) message implementation
- Ten FIPA interaction protocols: Request, Query, Contract-Net, Subscribe, Propose, English Auction, Dutch Auction, Brokering, Recruiting, and Iterated Contract-Net
- Formal AMS (Agent Management System) and DF (Directory Facilitator) platform agents
- Content language support with FIPA-SL codec and ontology validation

**JADE-Inspired Architecture**
- Behavior-based agent programming model (OneShotBehaviour, CyclicBehaviour, TickerBehaviour, FSMBehaviour, SequentialBehaviour, ParallelBehaviour)
- Yellow pages service discovery through the Directory Facilitator
- Agent lifecycle management via the Agent Management System
- Comprehensive monitoring tools including web dashboard, CLI, TUI, and message sniffer

**Modern Distributed Infrastructure**
- WebAssembly Component Model for portable, sandboxed agent execution
- libp2p peer-to-peer networking with Kademlia DHT, GossipSub, and NAT traversal
- Raft consensus for distributed state consistency
- Agent migration and cloning with full state preservation

**Enterprise Capabilities**
- Role-based access control with policy engine
- Agent authentication via certificates and tokens
- Persistent snapshots with crash recovery
- Inter-platform communication via pluggable Message Transport Protocols (HTTP, with extensibility for IIOP and others)

## Technical Specifications

| Component | Implementation |
|-----------|----------------|
| Language | Rust (2024 Edition) |
| Agent Runtime | Wasmtime 40 with Component Model |
| Networking | libp2p 0.56 (TCP, QUIC, Noise, Yamux) |
| Consensus | OpenRaft 0.9 |
| API | gRPC via Tonic, REST via Axum |
| Storage | Sled embedded database |

## Project Statistics

- **Test Coverage**: 174 unit tests across all modules
- **Modules**: 12 major subsystems (actor, behavior, protocol, platform, content, security, persistence, interplatform, consensus, network, observability, tools)
- **License**: MIT
- **Repository**: https://github.com/greenpdx/fipa-wasm

## Use Cases

The system is designed for building distributed multi-agent applications including:
- Autonomous negotiation and trading systems
- Distributed task coordination and workflow orchestration
- IoT device networks with intelligent edge agents
- Federated service discovery and brokering
- Simulation environments for agent-based modeling

## Acknowledgments

This project builds upon the foundational work of the FIPA organization in standardizing agent communication and the JADE team's practical implementation experience. The behavior model, platform agent architecture, and content language framework are directly inspired by JADE's design, adapted for the Rust ecosystem with modern async/await patterns and WebAssembly portability.

We welcome collaboration and feedback from the agent systems research community.

---

**Contact**: SavageS
**Project**: fipa-wasm-agents v0.2.0
**Date**: December 2024
