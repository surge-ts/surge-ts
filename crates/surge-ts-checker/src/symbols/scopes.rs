use std::collections::HashMap;
use std::sync::Arc;

use crate::program::record_scope_stack_visible_symbol_handle_copy_count;
use surge_ts_syntax::ParsedExpression;
use surge_ts_types::Type;

use crate::symbols::{
    AutoArrayBinding, SymbolInfo, SymbolInfoHandle, SymbolTable, TupleDestructureBinding,
    clone_symbol_info_handle,
};

#[derive(Debug, Clone)]
pub(crate) struct ScopeStack {
    frames: Vec<ScopeFrame>,
    visible_symbols: SymbolTable,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ScopeFrame {
    symbols: SymbolTable,
    visible_shadows: HashMap<Arc<str>, Option<SymbolInfoHandle>>,
    /// Declared types this frame's flow narrowing shadowed, restored on
    /// `pop_child` alongside `visible_shadows`. Only the narrowing paths write
    /// here, so it is empty in nearly every frame.
    declared_shadows: HashMap<Arc<str>, Option<Type>>,
    /// Guard-alias conditions this frame's declarations shadowed, restored on
    /// `pop_child` like `declared_shadows`.
    alias_shadows: HashMap<Arc<str>, Option<Arc<ParsedExpression>>>,
    destructure_shadows: HashMap<Arc<str>, Option<TupleDestructureBinding>>,
    /// Names this frame *declares*, as opposed to bindings of an enclosing
    /// frame it only flow-narrowed. `else { const v = … }` shadows a narrowed
    /// `v`, and that local must not be mistaken for the outer binding's state.
    declared_here: std::collections::HashSet<Arc<str>>,
    /// Evolving-array entries this frame's declarations shadowed. Only a
    /// declaration writes here; the mutations that grow an entry later persist
    /// past the frame, as element types only ever grow along the flow.
    auto_shadows: HashMap<Arc<str>, Option<AutoArrayBinding>>,
    /// A function body's own frame, which `var` hoisting stops at. Every other
    /// frame is a block (`{}`, an `if` branch, a loop or a `switch` case), and a
    /// `var` declared there stays visible after it.
    is_function_scope: bool,
}

impl ScopeStack {
    pub(crate) fn from_root(root: SymbolTable) -> Self {
        // Share the root (module/ambient) symbols through `parent` fallback rather
        // than as the own map of either table. Both `visible_symbols` and the root
        // frame are mutated on every local binding; holding `root` as their own
        // map made the first insert copy the entire module symbol table per
        // function body (the copy-on-write deep copy fired because the Arc was
        // shared). With the fallback, inserts hit small unshared own maps and
        // lookups still fall through to the full root. The fallback table is only
        // ever read via `get`, never iterated.
        let root = Arc::new(root);
        Self {
            visible_symbols: SymbolTable::with_parent(root.clone()),
            frames: vec![ScopeFrame {
                symbols: SymbolTable::with_parent(root),
                visible_shadows: HashMap::new(),
                declared_shadows: HashMap::new(),
                alias_shadows: HashMap::new(),
                destructure_shadows: HashMap::new(),
                declared_here: std::collections::HashSet::new(),
                auto_shadows: HashMap::new(),
                is_function_scope: true,
            }],
        }
    }

    #[allow(dead_code)]
    // Reserved for upcoming scope-aware reads when the checker starts using
    // nested scope lookups outside the current function-body flow path.
    pub(crate) fn resolve(&self, name: &str) -> Option<&SymbolInfo> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.symbols.get(name))
    }

    /// Whether a frame of this function body declares `name` itself, shadowing
    /// the module binding of the same name.
    pub(crate) fn declares_locally(&self, name: &str) -> bool {
        self.frames.iter().any(|frame| frame.symbols.get_own(name).is_some())
    }

    /// tsc binds a body-level `let`/`const`/`var` that repeats a parameter's
    /// name into the function's own locals, where the parameter's symbol stays
    /// (the binder reports TS2300): reads keep resolving to the parameter.
    fn keeps_parameter(&self, name: &str, incoming: &SymbolInfo) -> bool {
        matches!(
            incoming.kind,
            crate::symbols::SymbolKind::Var | crate::symbols::SymbolKind::Let | crate::symbols::SymbolKind::Const
        ) && self.frames.last().is_some_and(|frame| {
            // A `catch` clause's binding is a block's; a `var` there is the
            // function's own variable.
            frame.is_function_scope
                && frame
                    .symbols
                    .get_own(name)
                    .is_some_and(|existing| matches!(existing.kind, crate::symbols::SymbolKind::Parameter))
        })
    }

    pub(crate) fn insert_current(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfo,
    ) -> Option<SymbolInfoHandle> {
        self.insert_current_handle(name, Arc::new(symbol))
    }

    /// Install a flow-narrowed type, remembering the type it was narrowed *from*
    /// so an assignment inside the branch still checks against the declaration
    /// (`if (v === undefined) { v = "x" }` where `v: string | undefined`).
    pub(crate) fn insert_current_narrowed(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfo,
        declared: Type,
    ) -> Option<SymbolInfoHandle> {
        let name = name.into();
        if self.keeps_parameter(&name, &symbol) {
            return None;
        }
        let previous_declared = self.visible_symbols.declared_type(&name).cloned();
        let current_frame = self
            .frames
            .last_mut()
            .expect("scope stack must contain at least one frame");
        current_frame
            .declared_shadows
            .entry(Arc::clone(&name))
            .or_insert(previous_declared.clone());
        self.visible_symbols.set_declared_type(
            Arc::clone(&name),
            Some(previous_declared.unwrap_or(declared)),
        );
        self.insert_frame_entry(name, Arc::new(symbol))
    }

    /// Replaces a binding's flow type without declaring anything: what an
    /// assignment or a guard does to a binding some enclosing frame declared.
    pub(crate) fn insert_current_flow(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfo,
    ) -> Option<SymbolInfoHandle> {
        self.insert_frame_entry(name.into(), Arc::new(symbol))
    }

    /// Declares `name` in the current frame with `declared` as its declaration
    /// type. Unlike a narrowing, a declaration does not inherit the record of
    /// an outer binding it shadows: `let v: T` in a block, where the block
    /// declares its own `T`, is checked against *that* `T`.
    pub(crate) fn insert_current_declared(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfo,
        declared: Type,
    ) -> Option<SymbolInfoHandle> {
        let name = name.into();
        if self.keeps_parameter(&name, &symbol) {
            return None;
        }
        let previous_declared = self.visible_symbols.declared_type(&name).cloned();
        let current_frame = self
            .frames
            .last_mut()
            .expect("scope stack must contain at least one frame");
        current_frame
            .declared_shadows
            .entry(Arc::clone(&name))
            .or_insert(previous_declared);
        self.visible_symbols
            .set_declared_type(Arc::clone(&name), Some(declared));
        self.insert_current(name, symbol)
    }

    /// Records (or clears, with `None`) the condition a boolean `const` guard
    /// stands for, visible to expression-level narrowing until the frame pops.
    pub(crate) fn record_alias_condition(
        &mut self,
        name: &str,
        condition: Option<Arc<ParsedExpression>>,
    ) {
        let name: Arc<str> = name.into();
        let previous = self.visible_symbols.alias_condition(&name);
        if previous.is_none() && condition.is_none() {
            return;
        }
        let current_frame = self
            .frames
            .last_mut()
            .expect("scope stack must contain at least one frame");
        current_frame
            .alias_shadows
            .entry(Arc::clone(&name))
            .or_insert(previous);
        self.visible_symbols.set_alias_condition(name, condition);
    }

    /// Records (or clears, with `None`) the tuple element a `const` binding was
    /// destructured from, until the frame pops.
    pub(crate) fn record_tuple_destructure(
        &mut self,
        name: &str,
        binding: Option<TupleDestructureBinding>,
    ) {
        let name: Arc<str> = name.into();
        let previous = self.visible_symbols.tuple_destructure(&name);
        if previous.is_none() && binding.is_none() {
            return;
        }
        let current_frame = self
            .frames
            .last_mut()
            .expect("scope stack must contain at least one frame");
        current_frame
            .destructure_shadows
            .entry(Arc::clone(&name))
            .or_insert(previous);
        self.visible_symbols.set_tuple_destructure(name, binding);
    }

    /// Records whether the binding just declared as `name` is an evolving
    /// array, hiding any outer entry of the same name until the frame pops.
    pub(crate) fn declare_auto_array(&mut self, name: &str, binding: Option<AutoArrayBinding>) {
        if binding.is_none() && !self.visible_symbols.has_auto_arrays() {
            return;
        }
        let name: Arc<str> = name.into();
        let previous = self
            .visible_symbols
            .set_auto_array(Arc::clone(&name), binding);
        let current_frame = self
            .frames
            .last_mut()
            .expect("scope stack must contain at least one frame");
        current_frame.auto_shadows.entry(name).or_insert(previous);
    }

    pub(crate) fn auto_array_mut(&mut self, name: &str) -> Option<&mut AutoArrayBinding> {
        self.visible_symbols.auto_array_mut(name)
    }

    pub(crate) fn insert_current_handle(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfoHandle,
    ) -> Option<SymbolInfoHandle> {
        let name = name.into();
        if self.keeps_parameter(&name, &symbol) {
            return None;
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.declared_here.insert(Arc::clone(&name));
        }
        self.insert_frame_entry(name, symbol)
    }

    fn insert_frame_entry(
        &mut self,
        name: Arc<str>,
        symbol: SymbolInfoHandle,
    ) -> Option<SymbolInfoHandle> {
        // The displaced entry is the visible table's *own* one: a binding it
        // only reaches through its parent (a closure over the enclosing scope)
        // is restored by removing the copy, which leaves the parent's entry —
        // and the declared type recorded beside it — in charge again.
        let previous_visible = self.visible_symbols.get_own_shared(&name);
        let current_frame = self
            .frames
            .last_mut()
            .expect("scope stack must contain at least one frame");
        if current_frame.symbols.get(&name).is_none() {
            current_frame
                .visible_shadows
                .insert(name.clone(), previous_visible);
        }

        record_scope_stack_visible_symbol_handle_copy_count(1);
        self.visible_symbols
            .insert_handle(name.clone(), clone_symbol_info_handle(&symbol));
        current_frame.symbols.insert_handle(name, symbol)
    }

    pub(crate) fn update_visible(&mut self, name: &str, symbol: SymbolInfo) -> bool {
        self.update_visible_handle(name, Arc::new(symbol))
    }

    /// [`Self::update_visible`] for a type the binding was narrowed to: when
    /// nothing recorded its declaration yet, `declared` is kept as that.
    pub(crate) fn update_visible_narrowed(&mut self, name: &str, symbol: SymbolInfo, declared: Type) -> bool {
        if self.visible_symbols.declared_type(name).is_none() {
            let name: Arc<str> = name.into();
            if let Some(frame) = self.frames.iter_mut().rev().find(|frame| frame.symbols.get(&name).is_some())
                && frame.symbols.declared_type(&name).is_none()
            {
                frame.symbols.set_declared_type(Arc::clone(&name), Some(declared.clone()));
            }
            self.visible_symbols.set_declared_type(name, Some(declared));
        }
        self.update_visible(name, symbol)
    }

    pub(crate) fn update_visible_handle(&mut self, name: &str, symbol: SymbolInfoHandle) -> bool {
        let name: Arc<str> = name.into();
        for frame in self.frames.iter_mut().rev() {
            if frame.symbols.get(&name).is_some() {
                record_scope_stack_visible_symbol_handle_copy_count(1);
                // The new type narrows the same binding, so its declaration
                // must stay readable from the entry that now answers for it.
                let declared = self.visible_symbols.declared_type(&name).cloned();
                self.visible_symbols
                    .insert_handle(name.clone(), clone_symbol_info_handle(&symbol));
                if declared.is_some() && self.visible_symbols.declared_type(&name).is_none() {
                    self.visible_symbols.set_declared_type(name.clone(), declared.clone());
                }
                let frame_declared = frame.symbols.declared_type(&name).cloned().or(declared);
                frame.symbols.insert_handle(name.clone(), symbol);
                if frame_declared.is_some() && frame.symbols.declared_type(&name).is_none() {
                    frame.symbols.set_declared_type(name, frame_declared);
                }
                return true;
            }
        }

        false
    }

    pub(crate) fn current_contains_let_or_const(&self, name: &str) -> bool {
        self.frames
            .last()
            .is_some_and(|frame| frame.symbols.contains_let_or_const(name))
    }

    /// Marks this stack as a function body's, whose reads of an enclosing
    /// binding leave its flow.
    pub(crate) fn mark_function_boundary(&mut self) {
        self.visible_symbols.mark_function_boundary();
    }

    pub(crate) fn push_child(&mut self) {
        self.frames.push(ScopeFrame::default());
    }

    /// A function body's own frame. `var` hoisting stops here, and the parameter
    /// bindings it holds must not escape into the enclosing body.
    pub(crate) fn push_function_scope(&mut self) {
        self.frames.push(ScopeFrame {
            is_function_scope: true,
            ..ScopeFrame::default()
        });
    }

    /// The bindings the current frame flow-narrowed — not the ones it declared —
    /// with the type each was narrowed from. When this frame is the only branch
    /// of an `if` that can complete, they are what the code after it sees.
    pub(crate) fn current_frame_narrowings(&self) -> Vec<(Arc<str>, SymbolInfoHandle, Type)> {
        let Some(frame) = self.frames.last() else {
            return Vec::new();
        };
        frame
            .declared_shadows
            .keys()
            .filter(|name| !frame.declared_here.contains(*name))
            .filter_map(|name| {
                let symbol = self.visible_symbols.get_handle(name)?;
                let declared = self.visible_symbols.declared_type(name)?.clone();
                Some((Arc::clone(name), symbol, declared))
            })
            .collect()
    }

    /// Installs narrowings taken from a branch frame into the current one.
    pub(crate) fn adopt_narrowings(&mut self, narrowings: Vec<(Arc<str>, SymbolInfoHandle, Type)>) {
        for (name, symbol, declared) in narrowings {
            let _ = self.insert_current_narrowed(name, (*symbol).clone(), declared);
        }
    }

    pub(crate) fn pop_child(&mut self) {
        assert!(
            !self.frames.is_empty(),
            "scope stack must contain at least one frame"
        );
        let frame = self.frames.pop().expect("scope stack must contain a frame");
        for (name, previous_declared) in frame.declared_shadows {
            self.visible_symbols
                .set_declared_type(name, previous_declared);
        }
        for (name, previous_binding) in frame.auto_shadows {
            self.visible_symbols.set_auto_array(name, previous_binding);
        }
        for (name, previous_condition) in frame.alias_shadows {
            self.visible_symbols
                .set_alias_condition(name, previous_condition);
        }
        for (name, previous_binding) in frame.destructure_shadows {
            self.visible_symbols
                .set_tuple_destructure(name, previous_binding);
        }
        for (name, previous_symbol) in frame.visible_shadows {
            // `var` is function-scoped: a block that declared one leaves it
            // visible after the block, and the shadow it displaced moves up so
            // the enclosing frame restores it instead. A block that only
            // narrowed an enclosing `var` (a `case` clause of `switch (v)`)
            // restores it like any other binding.
            if !frame.is_function_scope
                && frame.declared_here.contains(&name)
                && self
                    .visible_symbols
                    .get(&name)
                    .is_some_and(|symbol| matches!(symbol.kind, crate::symbols::SymbolKind::Var))
            {
                if let Some(parent) = self.frames.last_mut() {
                    parent
                        .visible_shadows
                        .entry(name)
                        .or_insert(previous_symbol);
                    continue;
                }
            }
            match previous_symbol {
                Some(previous_symbol) => {
                    record_scope_stack_visible_symbol_handle_copy_count(1);
                    self.visible_symbols.insert_handle(name, previous_symbol);
                }
                None => {
                    self.visible_symbols.remove(&name);
                }
            }
        }
    }

    pub(crate) fn visible_symbols(&self) -> &SymbolTable {
        &self.visible_symbols
    }
}
