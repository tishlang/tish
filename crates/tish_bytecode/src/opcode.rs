//! Bytecode opcodes for the Tish VM.

/// Stack-based bytecode opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    /// No operation
    Nop = 0,
    /// Push constant from constants table (operand: u16 index)
    LoadConst = 1,
    /// Load variable from scope (operand: u16 name index)
    LoadVar = 2,
    /// Store top of stack to variable (operand: u16 name index)
    StoreVar = 3,
    /// Discard top of stack
    Pop = 4,
    /// Duplicate top of stack
    Dup = 5,
    /// Call function with n args (operand: u16 arg count). Callee and args on stack.
    Call = 6,
    /// Return from function. Top of stack is return value.
    Return = 7,
    /// Unconditional jump forward (operand: u16 byte offset)
    Jump = 8,
    /// Pop top; if falsy, jump forward (operand: u16 byte offset)
    JumpIfFalse = 9,
    /// Unconditional jump backward (operand: u16 byte offset)
    JumpBack = 10,
    /// Binary operation (operand: u8 BinOp variant). Pops left, right; pushes result.
    BinOp = 11,
    /// Unary operation (operand: u8 UnaryOp variant). Pops operand; pushes result.
    UnaryOp = 12,
    /// Get property: obj.prop (operand: u16 prop name index). Pops obj; pushes value.
    GetMember = 13,
    /// Set property: obj.prop = val (operand: u16 prop name index). Pops obj, val.
    SetMember = 14,
    /// Get index: obj[idx]. Pops obj, idx; pushes value.
    GetIndex = 15,
    /// Set index: obj[idx] = val. Pops obj, idx, val.
    SetIndex = 16,
    /// Create array with n elements (operand: u16 count). Elements on stack.
    NewArray = 17,
    /// Create object with n key-value pairs (operand: u16 count). Keys and vals interleaved.
    NewObject = 18,
    /// Load from global scope (operand: u16 name index)
    LoadGlobal = 19,
    /// Store to global scope (operand: u16 name index)
    StoreGlobal = 20,
    /// Create closure: push function (operand: u16 chunk index for nested function)
    Closure = 21,
    /// Pop and discard n values (operand: u16 count)
    PopN = 22,
    /// Load `this` or receiver (for method calls)
    LoadThis = 23,
    /// Throw: pop value, unwind to catch handler, push value, jump
    Throw = 24,
    /// EnterTry: push handler (catch offset u16). Catch offset = bytes from end of this insn.
    EnterTry = 25,
    /// ExitTry: pop try handler
    ExitTry = 26,
    /// Concat arrays: pop right, pop left, push left.concat(right). For spread.
    ConcatArray = 27,
    /// Merge objects: pop right, pop left, push Object.assign({}, left, right). For object spread.
    MergeObject = 28,
    /// Call with spread: pop args array, pop callee, call callee(...args).
    CallSpread = 29,
    /// Get property optional: like GetMember but returns Null if obj is null or prop missing.
    GetMemberOptional = 30,
    /// Pop array, sort numerically in place (operand: u8 0=asc, 1=desc), push array.
    /// Fast path for arr.sort((a,b)=>a-b) / arr.sort((a,b)=>b-a).
    ArraySortNumeric = 31,
    /// Pop array, sort by numeric property (operands: u16 prop_name_const_idx, u16 0=asc/1=desc).
    /// Fast path for arr.sort((a,b)=>a.prop-b.prop).
    ArraySortByProperty = 32,
    /// arr.map(x => x) - identity, returns array clone.
    ArrayMapIdentity = 33,
    /// arr.map(x => x op const) or arr.map(x => const op x). Operands: u8 binop, u16 const_idx, u8 param_left (0=param on left e.g. x*2, 1=param on right e.g. 2*x).
    ArrayMapBinOp = 34,
    /// arr.filter(x => x op const) or arr.filter(x => const op x). Operands: u8 binop, u16 const_idx, u8 param_left. Keeps elements where result is truthy.
    ArrayFilterBinOp = 35,
    /// Load built-in module export. Operands: u16 spec_const_idx, u16 export_name_const_idx. Pushes Value.
    LoadNativeExport = 36,
    /// `new callee(...args)` (operand: u16 arg count). Stack: callee, then args — same as Call.
    Construct = 37,
    /// `new callee(...spread)` — stack: args array, then callee (same order as CallSpread).
    ConstructSpread = 38,
    /// Declare `let`/`const` in the current lexical frame (operand: u16 name index). Pops value.
    /// Does not walk enclosing scopes or globals (unlike [`StoreVar`]).
    DeclareVar = 39,
    /// Enter a block scope; pairs with [`ExitBlock`].
    EnterBlock = 40,
    /// Exit innermost block scope and restore shadowed bindings.
    ExitBlock = 41,
    /// Like [`DeclareVar`] but does not record block-scope undo (for `for`/`for-of` header bindings).
    DeclareVarPlain = 42,
    /// Pop the `await` operand value; if it is a `Promise`, block until settled, push the result,
    /// or unwind to `catch` like `Throw` on rejection.
    AwaitPromise = 43,
    /// Load a local variable by frame slot (operand: u16 slot). Fast path: direct
    /// index into the current call frame's locals, no name lookup.
    LoadLocal = 44,
    /// Store top of stack into a local frame slot (operand: u16 slot). Leaves nothing.
    StoreLocal = 45,
    /// Load a captured variable from an enclosing frame (operands: u16 hops, u16 slot).
    /// Walks `hops` parent frames, then indexes `slot`.
    LoadUpvalue = 46,
    /// Store top of stack into an enclosing frame slot (operands: u16 hops, u16 slot).
    StoreUpvalue = 47,
    /// Begin a per-iteration binding region for a loop variable (operand: u16 name index).
    /// Registers the name so closures created in the loop body snapshot it into a fresh overlay
    /// (ES `let` per-iteration semantics); the rest of the frame stays shared. Emitted only when
    /// the loop body creates a closure, so closure-free (hot) loops are untouched.
    LoopVarsBegin = 48,
    /// End the innermost per-iteration binding region (no operand).
    LoopVarsEnd = 49,
    /// Push `Bool(param_index >= argc)` — true when the positional argument at `param_index`
    /// was not supplied by the caller (operand: u16 param index). Emitted by the function
    /// prologue so default parameter values apply only for *missing* args, matching the
    /// interpreter: an explicit `null` argument does NOT trigger the default.
    ArgMissing = 50,
    /// Direct self-recursive call (operand: u16 arg count). Emitted by the compiler ONLY when a
    /// `fn NAME` body calls `NAME(args)` and `NAME` is provably the function itself (not shadowed
    /// by a param/local, not reassigned anywhere in the body). Args are on the stack as for `Call`,
    /// but the callee is implicitly the currently-executing chunk — no name lookup, no closure
    /// dispatch. The numeric JIT lowers this to a native recursive call (the big recursion win);
    /// the VM runs the current chunk directly. Behaviour is identical to `LoadVar NAME; Call argc`.
    SelfCall = 51,
    /// Normalize the top-of-stack iterable for `for…of`: a JS iterator object (one with a
    /// callable `next()` returning `{ value, done }`, e.g. a `Map`/`Set` `.values()` result)
    /// is drained into an array; arrays/strings/anything else pass through unchanged. Emitted
    /// right after the iterable expression so the existing index-based loop can iterate it.
    IterNormalize = 52,
    /// `delete obj[key]` / `delete obj.prop`. Pops `[obj, key]`, removes the property
    /// (objects: drop the key; arrays: set the index to a null hole), pushes `true`.
    DeleteIndex = 53,
    /// String-builder append for statement-position `acc += rhs` where `acc` is a frame slot local
    /// (operand: u16 slot index). Pops `rhs`; appends it to the accumulator in amortized O(1) by
    /// keeping a growable buffer for the slot (see the frame-local builder in `run_chunk`). Leaves
    /// nothing on the stack — only emitted where the assignment's result is discarded. Slots are
    /// frame-private (never captured), so the buffer needs no cross-frame sharing.
    AppendLocal = 54,
    /// #186 — apply a unary `Math.<fn>` intrinsic to the top-of-stack number: pop one value, push
    /// `Math.fn(x)`. The u16 operand is the [`MathUnaryFn`] id. Emitted by the compiler ONLY when
    /// `Math` is provably the global builtin (never shadowed in the program), so the numeric JIT can
    /// lower it to a native op / libcall without a runtime shape guard. Behaviour is identical to
    /// `LoadVar Math; GetMember fn; <arg>; Call 1` on a number.
    MathUnary = 55,
}

/// The unary `Math` functions the [`Opcode::MathUnary`] fast path recognizes (#186). f64→f64;
/// the discriminant is the opcode operand. Kept in sync with the VM handler and the JIT lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MathUnaryFn {
    Sqrt = 0,
    Cbrt = 1,
    Abs = 2,
    Floor = 3,
    Ceil = 4,
    Round = 5,
    Trunc = 6,
    Sign = 7,
    Sin = 8,
    Cos = 9,
    Tan = 10,
    Asin = 11,
    Acos = 12,
    Atan = 13,
    Sinh = 14,
    Cosh = 15,
    Tanh = 16,
    Exp = 17,
    Log = 18,
    Log2 = 19,
    Log10 = 20,
}

impl MathUnaryFn {
    /// Map a `Math.<name>` member to its id, or `None` if it isn't a supported unary intrinsic.
    pub fn from_name(name: &str) -> Option<MathUnaryFn> {
        Some(match name {
            "sqrt" => MathUnaryFn::Sqrt,
            "cbrt" => MathUnaryFn::Cbrt,
            "abs" => MathUnaryFn::Abs,
            "floor" => MathUnaryFn::Floor,
            "ceil" => MathUnaryFn::Ceil,
            "round" => MathUnaryFn::Round,
            "trunc" => MathUnaryFn::Trunc,
            "sign" => MathUnaryFn::Sign,
            "sin" => MathUnaryFn::Sin,
            "cos" => MathUnaryFn::Cos,
            "tan" => MathUnaryFn::Tan,
            "asin" => MathUnaryFn::Asin,
            "acos" => MathUnaryFn::Acos,
            "atan" => MathUnaryFn::Atan,
            "sinh" => MathUnaryFn::Sinh,
            "cosh" => MathUnaryFn::Cosh,
            "tanh" => MathUnaryFn::Tanh,
            "exp" => MathUnaryFn::Exp,
            "log" => MathUnaryFn::Log,
            "log2" => MathUnaryFn::Log2,
            "log10" => MathUnaryFn::Log10,
            _ => return None,
        })
    }

    /// Decode an opcode operand to the fn id.
    pub fn from_u16(v: u16) -> Option<MathUnaryFn> {
        if v <= 20 {
            Some(unsafe { std::mem::transmute::<u16, MathUnaryFn>(v) })
        } else {
            None
        }
    }

    /// Apply the intrinsic (the single source of truth for VM + JIT + interp result parity).
    #[inline]
    pub fn apply(self, x: f64) -> f64 {
        match self {
            MathUnaryFn::Sqrt => x.sqrt(),
            MathUnaryFn::Cbrt => x.cbrt(),
            MathUnaryFn::Abs => x.abs(),
            MathUnaryFn::Floor => x.floor(),
            MathUnaryFn::Ceil => x.ceil(),
            // JS `Math.round`: ties toward +∞, `[-0.5, 0) → -0`; EXACTLY `tishlang_builtins::math::
            // round_f64` (replicated here — this crate sits below builtins — so VM/JIT == interp).
            MathUnaryFn::Round => {
                if x.is_nan() || x.is_infinite() || x == 0.0 {
                    x
                } else if (-0.5..0.5).contains(&x) {
                    if x < 0.0 {
                        -0.0
                    } else {
                        0.0
                    }
                } else {
                    (x + 0.5).floor()
                }
            }
            MathUnaryFn::Trunc => x.trunc(),
            // JS `Math.sign` (matches `tishlang_builtins::math::sign`): NaN→NaN, else ±1, and 0/-0→+0.
            MathUnaryFn::Sign => {
                if x.is_nan() {
                    f64::NAN
                } else if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            MathUnaryFn::Sin => x.sin(),
            MathUnaryFn::Cos => x.cos(),
            MathUnaryFn::Tan => x.tan(),
            MathUnaryFn::Asin => x.asin(),
            MathUnaryFn::Acos => x.acos(),
            MathUnaryFn::Atan => x.atan(),
            MathUnaryFn::Sinh => x.sinh(),
            MathUnaryFn::Cosh => x.cosh(),
            MathUnaryFn::Tanh => x.tanh(),
            MathUnaryFn::Exp => x.exp(),
            MathUnaryFn::Log => x.ln(),
            MathUnaryFn::Log2 => x.log2(),
            MathUnaryFn::Log10 => x.log10(),
        }
    }
}

impl Opcode {
    /// Decode byte to opcode. Safe for b in 0..=55 (matches #[repr(u8)] discriminants).
    #[inline]
    pub fn from_u8(b: u8) -> Option<Opcode> {
        if b <= 55 {
            Some(unsafe { std::mem::transmute::<u8, Opcode>(b) })
        } else {
            None
        }
    }

    /// Size in bytes of this instruction at `ip` (including operands). Returns None if truncated.
    pub fn instruction_size(self, code: &[u8], ip: usize) -> Option<usize> {
        let size = match self {
            Opcode::Nop
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::Return
            | Opcode::ExitTry
            | Opcode::ConcatArray
            | Opcode::MergeObject
            | Opcode::GetIndex
            | Opcode::SetIndex
            | Opcode::Throw
            | Opcode::ArrayMapIdentity
            | Opcode::CallSpread
            | Opcode::ConstructSpread
            | Opcode::EnterBlock
            | Opcode::ExitBlock
            | Opcode::LoopVarsEnd
            | Opcode::IterNormalize
            | Opcode::DeleteIndex
            | Opcode::AwaitPromise => 1,
            Opcode::ArraySortByProperty
            | Opcode::ArrayMapBinOp
            | Opcode::ArrayFilterBinOp
            | Opcode::LoadNativeExport
            | Opcode::LoadUpvalue
            | Opcode::StoreUpvalue => 5,
            // LoadLocal / StoreLocal take a single u16 operand → 3 bytes (default).
            _ => 3,
        };
        if ip + size > code.len() {
            return None;
        }
        Some(size)
    }
}
