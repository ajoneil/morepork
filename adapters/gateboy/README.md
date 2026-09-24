# GateBoy Adapter

Beyond its DMG-vocabulary columns, the adapter declares 63 gate-level columns (`bus_addr`, `op_state`, `mcycle_phase`, `mask_pipe`, the ten-slot sprite store `oam{0..9}_{x,id,attr}`, the APU channel counters/flags `ch{1..4}_*`, `halted`, `irq_pending`, `dispatch_active`, `irq_latched`, `vram_addr`/`vram_data` and `apu_write_addr`/`apu_write_data`) as its own header `extension_fields` under subsystem `gateboy`, layer `internal`, which a profile requests with `[fields.extensions] gateboy = [...]`.
