;; chat_agent.wat — a minimal ping-pong chat agent.
;;
;; The node drives the agent through host calls:
;;   init()                       once at startup
;;   config(unl, body)            once to seed (vocabulary), then per message
;;
;; The agent is seeded with its PEER's id: the seed config carries the
;; vocabulary as its UNL (JSON, so it begins with '{') and the agent's DATA
;; block as its body — and that DATA block is the peer's id. The agent stashes
;; the peer id, then on every subsequent message bounces the message straight
;; back to the peer (up to a bounded number of volleys), then falls silent.
;;
;; "Picking the id of the other chat" is therefore just setting each agent's
;; DATA block: ping's DATA = "pong", pong's DATA = "ping".
(module
  (import "fipa:agent/messaging" "send-unl"
    (func $send (param i32 i32 i32 i32 i32 i32)))

  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 8192))  ;; host scratch; peer id lives at 0
  (global $count (mut i32) (i32.const 0))    ;; volleys sent
  (global $peer_len (mut i32) (i32.const 0)) ;; length of the peer id at offset 0

  (func (export "init"))
  (func (export "run") (result i32) (i32.const 1)) ;; non-zero => keep running

  ;; bump allocator: the host writes inbound (unl, body) here before config()
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $n)))
    (local.get $p))

  (func (export "config") (param $up i32) (param $ul i32) (param $bp i32) (param $bl i32)
    (if (i32.eq (i32.load8_u (local.get $up)) (i32.const 0x7b)) ;; '{' => vocabulary seed
      (then
        ;; remember the peer id (the seed body) at offset 0
        (memory.copy (i32.const 0) (local.get $bp) (local.get $bl))
        (global.set $peer_len (local.get $bl)))
      (else
        ;; a message: bounce it back to the peer, bounded
        (if (i32.lt_s (global.get $count) (i32.const 4))
          (then
            (global.set $count (i32.add (global.get $count) (i32.const 1)))
            (call $send
              (i32.const 0) (global.get $peer_len) ;; → peer id
              (local.get $up) (local.get $ul)       ;; bounce the UNL
              (local.get $bp) (local.get $bl))))))))  ;; and the body
