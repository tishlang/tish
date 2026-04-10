//! Peephole optimizations on bytecode (post-emission).
//! B2 from optimization plan: jump chaining, etc.

use std::collections::BTreeSet;

use crate::opcode::Opcode;
use crate::Chunk;

fn read_u16(code: &[u8], pos: usize) -> u16 {
    if pos + 1 >= code.len() {
        return 0;
    }
    let a = code[pos] as u16;
    let b = code[pos + 1] as u16;
    (a << 8) | b
}

fn read_i16(code: &[u8], pos: usize) -> i16 {
    read_u16(code, pos) as i16
}

fn write_u16(code: &mut [u8], pos: usize, v: u16) {
    if pos + 1 < code.len() {
        let bytes = v.to_be_bytes();
        code[pos] = bytes[0];
        code[pos + 1] = bytes[1];
    }
}

/// Size of instruction at `ip` in bytes. Returns None if invalid/truncated.
fn instruction_size(code: &[u8], ip: usize) -> Option<usize> {
    if ip >= code.len() {
        return None;
    }
    let opcode = Opcode::from_u8(code[ip])?;
    opcode.instruction_size(code, ip)
}

/// Advance past `Nop` bytes left by other peepholes (`Dup`+`Pop` → `Nop`+`Nop`, etc.).
/// Jump resolution must not treat a `Nop` run as the end of a chain, or we leave a jump
/// targeting the middle of padding while `chain_jumps` redirects another jump past it —
/// that misaligns `||` short-circuit when nested in an outer `if`.
fn skip_leading_nops(code: &[u8], mut ip: usize) -> usize {
    while ip < code.len() && Opcode::from_u8(code[ip]) == Some(Opcode::Nop) {
        ip += 1;
    }
    ip
}

/// After a branch lands at `ip`, follow only **unconditional** `Jump` instructions.
/// Must not follow `JumpIfFalse`: that opcode is conditional; treating it like `Jump`
/// breaks short-circuit codegen (e.g. `a === 1 || b === 2` inside `if (...)`).
fn skip_unconditional_jump_chain(code: &[u8], mut ip: usize) -> Option<usize> {
    ip = skip_leading_nops(code, ip);
    let mut visited = 0u32;
    const MAX_CHAIN: u32 = 1000;
    loop {
        if visited > MAX_CHAIN {
            return None;
        }
        visited += 1;
        if ip > code.len() {
            return None;
        }
        if ip == code.len() {
            return Some(ip);
        }
        let _ = instruction_size(code, ip)?;
        let op = Opcode::from_u8(code[ip])?;
        if op != Opcode::Jump {
            return Some(ip);
        }
        let offset = read_i16(code, ip + 1) as isize;
        ip = (ip as isize + 3 + offset).max(0) as usize;
        ip = skip_leading_nops(code, ip);
    }
}

/// For a `Jump` or `JumpIfFalse` at `jump_ip`, return the final IP after resolving the
/// taken branch and then skipping through any **unconditional** `Jump` chain only.
fn final_jump_target(code: &[u8], jump_ip: usize) -> Option<usize> {
    let _ = instruction_size(code, jump_ip)?;
    let op = Opcode::from_u8(code[jump_ip])?;
    let first_target = match op {
        Opcode::Jump | Opcode::JumpIfFalse => {
            let offset = read_i16(code, jump_ip + 1) as isize;
            (jump_ip as isize + 3 + offset).max(0) as usize
        }
        _ => return Some(jump_ip),
    };
    let first_target = skip_leading_nops(code, first_target);
    skip_unconditional_jump_chain(code, first_target)
}

/// Instruction boundaries from a linear scan (aligned bytecode from the compiler).
fn collect_insn_starts(code: &[u8]) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut ip = 0usize;
    while ip < code.len() {
        out.insert(ip);
        let sz = instruction_size(code, ip).unwrap_or(1);
        ip += sz;
    }
    out
}

/// Replace instruction at [ip..ip+len) with Nops (preserves length, no offset updates).
fn nop_out(code: &mut [u8], ip: usize, len: usize) {
    for i in 0..len {
        if ip + i < code.len() {
            code[ip + i] = Opcode::Nop as u8;
        }
    }
}

/// Remove redundant Dup + Pop (dup top then discard = no-op).
fn remove_dup_pop(code: &mut [u8]) {
    let mut ip = 0;
    while ip + 2 <= code.len() {
        if Opcode::from_u8(code[ip]) == Some(Opcode::Dup)
            && Opcode::from_u8(code[ip + 1]) == Some(Opcode::Pop)
        {
            nop_out(code, ip, 2);
        }
        ip += instruction_size(code, ip).unwrap_or(1);
    }
}

/// Replace no-op jumps (Jump with offset 0) with Nops.
fn remove_noop_jumps(code: &mut [u8]) {
    let mut ip = 0;
    while ip < code.len() {
        if Opcode::from_u8(code[ip]) == Some(Opcode::Jump) {
            let offset = read_u16(code, ip + 1);
            if offset == 0 {
                nop_out(code, ip, 3);
            }
        }
        ip += instruction_size(code, ip).unwrap_or(1);
    }
}

/// Apply jump chaining: if Jump/JumpIfFalse targets another jump, update to
/// jump directly to the final target.
fn chain_jumps(code: &mut [u8]) {
    let insn_starts = collect_insn_starts(code);
    let mut ip = 0;
    while ip < code.len() {
        let op = match Opcode::from_u8(code[ip]) {
            Some(o) => o,
            None => {
                ip += 1;
                continue;
            }
        };
        let size = match instruction_size(code, ip) {
            Some(s) => s,
            None => break,
        };
        match op {
            Opcode::Jump | Opcode::JumpIfFalse => {
                let current_offset = read_i16(code, ip + 1) as isize;
                let current_target = (ip as isize + 3 + current_offset).max(0) as usize;
                if let Some(final_target) = final_jump_target(code, ip) {
                    let target_ok = final_target == code.len()
                        || insn_starts.contains(&final_target);
                    if final_target != current_target && target_ok {
                        let new_offset = final_target as i32 - (ip + 3) as i32;
                        if (i16::MIN as i32..=i16::MAX as i32).contains(&new_offset) {
                            write_u16(code, ip + 1, (new_offset as i16) as u16);
                        }
                    }
                }
            }
            _ => {}
        }
        ip += size;
    }
}

/// Run peephole optimizations on a chunk (and nested chunks).
pub fn optimize(chunk: &mut Chunk) {
    remove_dup_pop(&mut chunk.code);
    remove_noop_jumps(&mut chunk.code);
    chain_jumps(&mut chunk.code);
    for nested in &mut chunk.nested {
        optimize(nested);
    }
}
