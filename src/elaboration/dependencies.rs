use crate::comptime::ComptimeValue;
use crate::diagnostics::Diagnostics;
use crate::elaboration::Elaborator;
use crate::parser::ast::{Binding, Phase};
use crate::semantic::hir::HirBinding;
use crate::semantic::symbols::{ScopeId, SymbolId, SymbolKind};
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkStatus {
    Pending,
    Active,
    Complete,
    Failed,
}

#[derive(Debug, Clone)]
pub(super) struct PendingBinding {
    pub binding: Binding,
    pub span: Span,
    pub defining_scope: ScopeId,
}

impl Elaborator<'_> {
    pub(super) fn ensure_binding_elaborated(
        &mut self,
        symbol: SymbolId,
    ) -> Result<HirBinding, ()> {
        let status = self
            .elaboration_status
            .get(&symbol)
            .copied()
            .ok_or(())?;

        match status {
            WorkStatus::Complete => {
                return self
                    .elaborated_bindings
                    .get(&symbol)
                    .cloned()
                    .ok_or(());
            }

            WorkStatus::Failed => return Err(()),

            WorkStatus::Active => {
                self.report_elaboration_cycle(symbol);
                return Err(());
            }

            WorkStatus::Pending => {}
        }

        self.elaboration_status
            .insert(symbol, WorkStatus::Active);

        self.elaboration_stack.push(symbol);

        let pending = self
            .pending_bindings
            .get(&symbol)
            .cloned()
            .ok_or(())?;

        let previous_scope = self.environment.switch_scope(pending.defining_scope);

        let result = self.analyze_binding(
            &pending.binding,
            pending.span,
            symbol,
        );

        self.environment.switch_scope(previous_scope);

        let popped = self.elaboration_stack.pop();
        debug_assert_eq!(popped, Some(symbol));

        match result {
            Some(hir) => {
                self.elaborated_bindings
                    .insert(symbol, hir.clone());

                self.elaboration_status
                    .insert(symbol, WorkStatus::Complete);

                Ok(hir)
            }

            None => {
                self.elaboration_status
                    .insert(symbol, WorkStatus::Failed);

                Err(())
            }
        }
    }

    pub(super) fn ensure_binding_evaluated(
        &mut self,
        symbol: SymbolId,
    ) -> Result<ComptimeValue, ()> {
        let status = self
            .evaluation_status
            .get(&symbol)
            .copied()
            .ok_or(())?;

        match status {
            WorkStatus::Complete => {
                return self.values
                    .get(symbol)
                    .cloned()
                    .ok_or(());
            }

            WorkStatus::Failed => return Err(()),

            WorkStatus::Active => {
                self.report_evaluation_cycle(symbol);
                return Err(());
            }

            WorkStatus::Pending => {}
        }

        let hir = self.ensure_binding_elaborated(symbol)?;

        let is_comptime = matches!(
            self.symbols.get(symbol).kind,
            SymbolKind::Binding {
                phase: Phase::Comptime,
                ..
            }
        );

        if !is_comptime {
            self.evaluation_status
                .insert(symbol, WorkStatus::Failed);

            return Err(());
        }

        self.evaluation_status
            .insert(symbol, WorkStatus::Active);

        self.evaluation_stack.push(symbol);

        let value = self.evaluate_expression(&hir.expression);

        let popped = self.evaluation_stack.pop();
        debug_assert_eq!(popped, Some(symbol));

        if value == ComptimeValue::Error {
            self.evaluation_status
                .insert(symbol, WorkStatus::Failed);

            return Err(());
        }

        self.values.insert(symbol, value.clone());

        self.evaluation_status.insert(symbol, WorkStatus::Complete);

        Ok(value)
    }

    pub(super) fn report_evaluation_cycle(&mut self, repeated: SymbolId) {
        let stack = self.evaluation_stack.clone();

        self.report_cycle(
            "Compile-time dependency cycle",
            "these compile-time values recursively depend on eath other",
            &stack,
            repeated,
        );
    }

    pub(super) fn report_elaboration_cycle(&mut self, repeated: SymbolId) {
        let stack = self.elaboration_stack.clone();

        self.report_cycle(
            "Binding elaboration cycle",
            "the compiler cannot determine these binding types",
            &stack,
            repeated,
        );
    }

    fn report_cycle(
        &mut self,
        title: &str,
        explanation: &str,
        stack: &[SymbolId],
        repeated: SymbolId,
    ) {
        let cycle_start = stack
            .iter()
            .position(|symbol| *symbol == repeated)
            .unwrap_or(0);

        let mut cycle = stack[cycle_start..].to_vec();
        cycle.push(repeated);

        let path = cycle
            .iter()
            .map(|symbol| self.symbols.get(*symbol).name.clone())
            .collect::<Vec<_>>()
            .join(" -> ");

        let repeated_symbol = self.symbols.get(repeated);

        let span = repeated_symbol
            .declaration_span
            .expect(
                "dependency cycles should only contain source bindings"
            );

        self.diagnostics.error(
            title,
            span,
            format!("{explanation}: {path}"),
        )
    }
}