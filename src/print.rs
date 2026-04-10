//! Elm pretty-printer.
//!
//! The formatting approach is inspired by [elm-format](https://github.com/avh4/elm-format)'s
//! "Box" model. The core idea:
//!
//! 1. **`is_multiline(expr)`** — eagerly check if an expression would produce
//!    multi-line output (block expressions like `case`/`if`/`let`/`lambda` are
//!    always multi-line; containers are multi-line if any child is).
//!
//! 2. **Containers adapt** — lists, tuples, applications, and operator chains
//!    switch to one-element-per-line formatting when any element is multi-line.
//!
//! 3. **Block expressions get parens in atomic position** — when a `case`/`if`/`let`
//!    appears as a function argument or infix operand, it's parenthesized.
//!    Multi-line parens put the closing `)` on its own line so the re-parser
//!    can find it.
//!
//! This produces idempotent output: `print(parse(print(parse(src)))) == print(parse(src))`.

use crate::comment::Comment;
use crate::declaration::{CustomType, Declaration, InfixDef, TypeAlias, ValueConstructor};
use crate::exposing::{ExposedItem, Exposing};
use crate::expr::{
    CaseBranch, Expr, Function, FunctionImplementation, LetDeclaration, RecordSetter, Signature,
};
use crate::file::ElmModule;
use crate::import::Import;
use crate::literal::Literal;
use crate::module_header::ModuleHeader;
use crate::node::Spanned;
use crate::operator::InfixDirection;
use crate::pattern::Pattern;
use crate::type_annotation::{RecordField, TypeAnnotation};

/// Configuration for the Elm printer.
#[derive(Clone, Debug)]
pub struct PrintConfig {
    /// Number of spaces per indentation level.
    pub indent_width: usize,
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self { indent_width: 4 }
    }
}

/// Pretty-printer for Elm AST nodes.
pub struct Printer {
    config: PrintConfig,
    buf: String,
    indent: usize,
}

impl Printer {
    pub fn new(config: PrintConfig) -> Self {
        Self {
            config,
            buf: String::new(),
            indent: 0,
        }
    }

    /// Print a complete Elm module to a string.
    pub fn print_module(mut self, module: &ElmModule) -> String {
        self.write_module(module);
        self.buf
    }

    /// Print any single node to a string using `write_*` methods.
    pub fn finish(self) -> String {
        self.buf
    }

    // ── Output helpers ───────────────────────────────────────────────

    fn write(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    fn write_char(&mut self, c: char) {
        self.buf.push(c);
    }

    fn newline(&mut self) {
        self.buf.push('\n');
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent * self.config.indent_width {
            self.buf.push(' ');
        }
    }

    fn indent(&mut self) {
        self.indent += 1;
    }

    fn dedent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
    }

    fn newline_indent(&mut self) {
        self.newline();
        self.write_indent();
    }

    // ── Module ───────────────────────────────────────────────────────

    pub fn write_module(&mut self, module: &ElmModule) {
        // Sort comments by source position.
        let mut comments: Vec<&Spanned<Comment>> = module.comments.iter().collect();
        comments.sort_by_key(|c| c.span.start.offset);

        // Build an ordered list of "anchors" — items with start offsets that
        // comments can be assigned relative to. Each anchor is either an import
        // or a declaration. A comment belongs before the first anchor whose
        // start offset is strictly greater than the comment's offset.
        let num_imports = module.imports.len();
        let num_decls = module.declarations.len();
        let total_anchors = num_imports + num_decls;

        // anchor_offsets[i] = start offset of anchor i
        let mut anchor_offsets: Vec<usize> = Vec::with_capacity(total_anchors);
        for imp in &module.imports {
            anchor_offsets.push(imp.span.start.offset);
        }
        for decl in &module.declarations {
            anchor_offsets.push(decl.span.start.offset);
        }

        // Assign comments to slots: one per anchor + one trailing slot.
        let mut anchor_comments: Vec<Vec<&Spanned<Comment>>> = vec![vec![]; total_anchors + 1];
        for c in &comments {
            let offset = c.span.start.offset;
            let slot = anchor_offsets
                .iter()
                .position(|&a| a > offset)
                .unwrap_or(total_anchors);
            anchor_comments[slot].push(c);
        }

        self.write_module_header(&module.header.value);
        self.newline();

        if !module.imports.is_empty() {
            self.newline();
            for (i, imp) in module.imports.iter().enumerate() {
                // Slot i = comments before import i.
                if !anchor_comments[i].is_empty() {
                    for c in &anchor_comments[i] {
                        self.write_comment(&c.value);
                        self.newline();
                    }
                }
                self.write_import(&imp.value);
                self.newline();
            }
        }

        for (i, decl) in module.declarations.iter().enumerate() {
            let slot = num_imports + i;

            // Blank line separator before each declaration.
            self.newline();

            // Emit leading comments for this declaration.
            if !anchor_comments[slot].is_empty() {
                self.newline();
                for c in &anchor_comments[slot] {
                    self.write_comment(&c.value);
                    self.newline();
                }
            } else {
                self.newline();
            }

            self.write_declaration(&decl.value);
        }

        // Trailing comments after the last anchor.
        if !anchor_comments[total_anchors].is_empty() {
            self.newline();
            self.newline();
            for c in &anchor_comments[total_anchors] {
                self.write_comment(&c.value);
                self.newline();
            }
        }

        self.newline();
    }

    fn write_comment(&mut self, comment: &Comment) {
        match comment {
            Comment::Line(text) => {
                self.write("--");
                self.write(text);
            }
            Comment::Block(text) => {
                self.write("{-");
                self.write(text);
                self.write("-}");
            }
            Comment::Doc(text) => {
                self.write("{-|");
                self.write(text);
                self.write("-}");
            }
        }
    }

    fn write_module_header(&mut self, header: &ModuleHeader) {
        match header {
            ModuleHeader::Normal { name, exposing } => {
                self.write("module ");
                self.write_module_name(&name.value);
                self.write(" exposing ");
                self.write_exposing(&exposing.value);
            }
            ModuleHeader::Port { name, exposing } => {
                self.write("port module ");
                self.write_module_name(&name.value);
                self.write(" exposing ");
                self.write_exposing(&exposing.value);
            }
            ModuleHeader::Effect {
                name,
                exposing,
                command,
                subscription,
            } => {
                self.write("effect module ");
                self.write_module_name(&name.value);
                self.write(" where { ");
                let mut entries = Vec::new();
                if let Some(cmd) = command {
                    entries.push(format!("command = {}", cmd.value));
                }
                if let Some(sub) = subscription {
                    entries.push(format!("subscription = {}", sub.value));
                }
                self.write(&entries.join(", "));
                self.write(" } exposing ");
                self.write_exposing(&exposing.value);
            }
        }
    }

    fn write_module_name(&mut self, parts: &[String]) {
        self.write(&parts.join("."));
    }

    // ── Exposing ─────────────────────────────────────────────────────

    fn write_exposing(&mut self, exposing: &Exposing) {
        match exposing {
            Exposing::All(_) => self.write("(..)"),
            Exposing::Explicit(items) => {
                self.write_char('(');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_exposed_item(&item.value);
                }
                self.write_char(')');
            }
        }
    }

    fn write_exposed_item(&mut self, item: &ExposedItem) {
        match item {
            ExposedItem::Function(name) => self.write(name),
            ExposedItem::TypeOrAlias(name) => self.write(name),
            ExposedItem::TypeExpose { name, open } => {
                self.write(name);
                if open.is_some() {
                    self.write("(..)");
                }
            }
            ExposedItem::Infix(op) => {
                self.write_char('(');
                self.write(op);
                self.write_char(')');
            }
        }
    }

    // ── Import ───────────────────────────────────────────────────────

    fn write_import(&mut self, import: &Import) {
        self.write("import ");
        self.write_module_name(&import.module_name.value);
        if let Some(alias) = &import.alias {
            self.write(" as ");
            self.write_module_name(&alias.value);
        }
        if let Some(exposing) = &import.exposing {
            self.write(" exposing ");
            self.write_exposing(&exposing.value);
        }
    }

    // ── Declarations ─────────────────────────────────────────────────

    pub fn write_declaration(&mut self, decl: &Declaration) {
        match decl {
            Declaration::FunctionDeclaration(func) => self.write_function(func),
            Declaration::AliasDeclaration(alias) => self.write_type_alias(alias),
            Declaration::CustomTypeDeclaration(ct) => self.write_custom_type(ct),
            Declaration::PortDeclaration(sig) => {
                self.write("port ");
                self.write_signature(sig);
            }
            Declaration::InfixDeclaration(infix) => self.write_infix_decl(infix),
            Declaration::Destructuring { pattern, body } => {
                self.write_pattern(&pattern.value);
                self.write(" =");
                self.indent();
                self.newline_indent();
                self.write_expr(&body.value);
                self.dedent();
            }
        }
    }

    fn write_function(&mut self, func: &Function) {
        if let Some(doc) = &func.documentation {
            self.write("{-|");
            self.write(&doc.value);
            self.write("-}");
            self.newline();
        }
        if let Some(sig) = &func.signature {
            self.write_signature(&sig.value);
            self.newline();
        }
        self.write_function_impl(&func.declaration.value);
    }

    fn write_signature(&mut self, sig: &Signature) {
        self.write(&sig.name.value);
        self.write(" : ");
        self.write_type(&sig.type_annotation.value);
    }

    fn write_function_impl(&mut self, imp: &FunctionImplementation) {
        self.write(&imp.name.value);
        for arg in &imp.args {
            self.write_char(' ');
            self.write_pattern_atomic(&arg.value);
        }
        self.write(" =");
        self.indent();
        self.newline_indent();
        self.write_expr(&imp.body.value);
        self.dedent();
    }

    fn write_type_alias(&mut self, alias: &TypeAlias) {
        if let Some(doc) = &alias.documentation {
            self.write("{-|");
            self.write(&doc.value);
            self.write("-}");
            self.newline();
        }
        self.write("type alias ");
        self.write(&alias.name.value);
        for g in &alias.generics {
            self.write_char(' ');
            self.write(&g.value);
        }
        self.write(" =");
        self.indent();
        self.newline_indent();
        self.write_type(&alias.type_annotation.value);
        self.dedent();
    }

    fn write_custom_type(&mut self, ct: &CustomType) {
        if let Some(doc) = &ct.documentation {
            self.write("{-|");
            self.write(&doc.value);
            self.write("-}");
            self.newline();
        }
        self.write("type ");
        self.write(&ct.name.value);
        for g in &ct.generics {
            self.write_char(' ');
            self.write(&g.value);
        }
        self.indent();
        for (i, ctor) in ct.constructors.iter().enumerate() {
            self.newline_indent();
            if i == 0 {
                self.write("= ");
            } else {
                self.write("| ");
            }
            self.write_value_constructor(&ctor.value);
        }
        self.dedent();
    }

    fn write_value_constructor(&mut self, ctor: &ValueConstructor) {
        self.write(&ctor.name.value);
        for arg in &ctor.args {
            self.write_char(' ');
            self.write_type_atomic(&arg.value);
        }
    }

    fn write_infix_decl(&mut self, infix: &InfixDef) {
        self.write("infix ");
        match infix.direction.value {
            InfixDirection::Left => self.write("left"),
            InfixDirection::Right => self.write("right"),
            InfixDirection::Non => self.write("non"),
        }
        self.write_char(' ');
        self.write(&infix.precedence.value.to_string());
        self.write(" (");
        self.write(&infix.operator.value);
        self.write(") = ");
        self.write(&infix.function.value);
    }

    // ── Type annotations ─────────────────────────────────────────────

    pub fn write_type(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::FunctionType { from, to } => {
                self.write_type_non_arrow(&from.value);
                self.write(" -> ");
                self.write_type(&to.value);
            }
            _ => self.write_type_non_arrow(ty),
        }
    }

    fn write_type_non_arrow(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::FunctionType { .. } => {
                self.write_char('(');
                self.write_type(ty);
                self.write_char(')');
            }
            TypeAnnotation::Typed {
                module_name,
                name,
                args,
            } => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(&name.value);
                for arg in args {
                    self.write_char(' ');
                    self.write_type_atomic(&arg.value);
                }
            }
            _ => self.write_type_atomic(ty),
        }
    }

    fn write_type_atomic(&mut self, ty: &TypeAnnotation) {
        match ty {
            TypeAnnotation::GenericType(name) => self.write(name),
            TypeAnnotation::Unit => self.write("()"),
            TypeAnnotation::Typed {
                module_name,
                name,
                args,
            } if args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(&name.value);
            }
            TypeAnnotation::Tupled(elems) => {
                self.write("( ");
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_type(&elem.value);
                }
                self.write(" )");
            }
            TypeAnnotation::Record(fields) => {
                self.write_record_type_fields(fields, None);
            }
            TypeAnnotation::GenericRecord { base, fields } => {
                self.write_record_type_fields(fields, Some(&base.value));
            }
            _ => {
                self.write_char('(');
                self.write_type(ty);
                self.write_char(')');
            }
        }
    }

    fn write_record_type_fields(&mut self, fields: &[Spanned<RecordField>], base: Option<&str>) {
        if fields.is_empty() && base.is_none() {
            self.write("{}");
            return;
        }
        self.write("{ ");
        if let Some(base_name) = base {
            self.write(base_name);
            self.write(" | ");
        }
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.write(&field.value.name.value);
            self.write(" : ");
            self.write_type(&field.value.type_annotation.value);
        }
        self.write(" }");
    }

    // ── Patterns ─────────────────────────────────────────────────────

    pub fn write_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::As { pattern, name } => {
                self.write_pattern_cons(&pattern.value);
                self.write(" as ");
                self.write(&name.value);
            }
            _ => self.write_pattern_cons(pat),
        }
    }

    fn write_pattern_cons(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Cons { head, tail } => {
                self.write_pattern_app(&head.value);
                self.write(" :: ");
                self.write_pattern_cons(&tail.value);
            }
            _ => self.write_pattern_app(pat),
        }
    }

    fn write_pattern_app(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Constructor {
                module_name,
                name,
                args,
            } if !args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(name);
                for arg in args {
                    self.write_char(' ');
                    self.write_pattern_atomic(&arg.value);
                }
            }
            _ => self.write_pattern_atomic(pat),
        }
    }

    fn write_pattern_atomic(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Anything => self.write_char('_'),
            Pattern::Var(name) => self.write(name),
            Pattern::Literal(lit) => self.write_literal(lit),
            Pattern::Unit => self.write("()"),
            Pattern::Hex(n) => {
                self.write("0x");
                self.write(&format!("{n:X}"));
            }
            Pattern::Tuple(elems) => {
                self.write("( ");
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_pattern(&elem.value);
                }
                self.write(" )");
            }
            Pattern::Constructor {
                module_name,
                name,
                args,
            } if args.is_empty() => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(name);
            }
            Pattern::Record(fields) => {
                self.write("{ ");
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(&field.value);
                }
                self.write(" }");
            }
            Pattern::List(elems) => {
                if elems.is_empty() {
                    self.write("[]");
                } else {
                    self.write("[ ");
                    for (i, elem) in elems.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.write_pattern(&elem.value);
                    }
                    self.write(" ]");
                }
            }
            Pattern::Parenthesized(inner) => {
                self.write_char('(');
                self.write_pattern(&inner.value);
                self.write_char(')');
            }
            Pattern::Constructor { .. } | Pattern::Cons { .. } | Pattern::As { .. } => {
                self.write_char('(');
                self.write_pattern(pat);
                self.write_char(')');
            }
        }
    }

    // ── Expressions ──────────────────────────────────────────────────

    /// Write an expression in a "top-level body" context where block
    /// expressions (case/if/let/lambda) can appear directly.
    pub fn write_expr(&mut self, expr: &Expr) {
        self.write_expr_inner(expr);
    }

    /// Emit leading comments attached to a node.
    fn write_leading_comments(&mut self, comments: &[Spanned<Comment>]) {
        for c in comments {
            self.write_comment(&c.value);
            self.newline();
            self.write_indent();
        }
    }

    /// The core expression dispatcher.
    fn write_expr_inner(&mut self, expr: &Expr) {
        match expr {
            Expr::OperatorApplication {
                operator,
                left,
                right,
                ..
            } => {
                self.write_leading_comments(&left.comments);
                self.write_expr_operand(&left.value, operator, true);
                self.write_char(' ');
                self.write(operator);
                self.write_char(' ');
                self.write_leading_comments(&right.comments);
                self.write_expr_operand(&right.value, operator, false);
            }
            Expr::IfElse {
                branches,
                else_branch,
            } => {
                self.write_if_expr(branches, &else_branch.value);
            }
            Expr::CaseOf {
                expr: subject,
                branches,
            } => {
                self.write_case_expr(&subject.value, branches);
            }
            Expr::LetIn { declarations, body } => {
                self.write_let_expr(declarations, &body.value);
            }
            Expr::Lambda { args, body } => {
                self.write_lambda(args, &body.value);
            }
            Expr::BinOps {
                operands_and_operators,
                final_operand,
            } => {
                for (operand, op) in operands_and_operators {
                    self.write_expr_app(&operand.value);
                    self.write_char(' ');
                    self.write(&op.value);
                    self.write_char(' ');
                }
                self.write_expr_app(&final_operand.value);
            }
            _ => self.write_expr_app(expr),
        }
    }

    /// Write an operator operand, adding parens for precedence.
    fn write_expr_operand(&mut self, expr: &Expr, parent_op: &str, is_left: bool) {
        match expr {
            Expr::OperatorApplication { operator, .. } => {
                let parent_prec = op_precedence(parent_op);
                let child_prec = op_precedence(operator);
                let needs_parens = child_prec < parent_prec
                    || (child_prec == parent_prec
                        && ((is_left && is_right_assoc(parent_op))
                            || (!is_left && !is_right_assoc(parent_op))));
                if needs_parens {
                    self.write_char('(');
                    self.write_expr_inner(expr);
                    self.write_char(')');
                } else {
                    self.write_expr_inner(expr);
                }
            }
            _ => self.write_expr_app(expr),
        }
    }

    /// Write a function application or negation.
    fn write_expr_app(&mut self, expr: &Expr) {
        match expr {
            Expr::Application(args) => {
                // Application args are always written on the same line.
                // Block expression args get parenthesized by write_expr_atomic.
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.write_char(' ');
                    }
                    self.write_expr_atomic(&arg.value);
                }
            }
            Expr::Negation(inner) => {
                self.write_char('-');
                self.write_expr_atomic(&inner.value);
            }
            _ => self.write_expr_atomic(expr),
        }
    }

    /// Write an expression in atomic (highest-precedence) position.
    /// Complex and block expressions get parenthesized.
    fn write_expr_atomic(&mut self, expr: &Expr) {
        match expr {
            Expr::Unit => self.write("()"),
            Expr::Literal(lit) => self.write_literal(lit),

            Expr::FunctionOrValue { module_name, name } => {
                if !module_name.is_empty() {
                    self.write(&module_name.join("."));
                    self.write_char('.');
                }
                self.write(name);
            }

            Expr::PrefixOperator(op) => {
                self.write_char('(');
                self.write(op);
                self.write_char(')');
            }

            Expr::Parenthesized(inner) => {
                self.write_char('(');
                self.write_expr(&inner.value);
                self.write_char(')');
            }

            Expr::Tuple(elems) => {
                self.write_comma_sep("( ", " )", elems);
            }

            Expr::List(elems) => {
                if elems.is_empty() {
                    self.write("[]");
                } else {
                    self.write_comma_sep("[ ", " ]", elems);
                }
            }

            Expr::Record(fields) => {
                if fields.is_empty() {
                    self.write("{}");
                } else {
                    let any_ml = fields.iter().any(|f| is_multiline(&f.value.value.value));
                    if any_ml {
                        self.write("{");
                        self.indent();
                        for (i, field) in fields.iter().enumerate() {
                            self.newline_indent();
                            if i == 0 {
                                self.write("  ");
                            } else {
                                self.write(", ");
                            }
                            self.write_record_setter(&field.value);
                        }
                        self.newline_indent();
                        self.write("}");
                        self.dedent();
                    } else {
                        self.write("{ ");
                        for (i, field) in fields.iter().enumerate() {
                            if i > 0 {
                                self.write(", ");
                            }
                            self.write_record_setter(&field.value);
                        }
                        self.write(" }");
                    }
                }
            }

            Expr::RecordUpdate { base, updates } => {
                self.write("{ ");
                self.write(&base.value);
                self.write(" | ");
                for (i, field) in updates.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write_record_setter(&field.value);
                }
                self.write(" }");
            }

            Expr::RecordAccess { record, field } => {
                self.write_expr_atomic(&record.value);
                self.write_char('.');
                self.write(&field.value);
            }

            Expr::RecordAccessFunction(name) => {
                self.write_char('.');
                self.write(name);
            }

            Expr::GLSLExpression(src) => {
                self.write("[glsl|");
                self.write(src);
                self.write("|]");
            }

            // Non-block complex expressions: simple inline parens.
            Expr::OperatorApplication { .. }
            | Expr::Application(_)
            | Expr::Negation(_)
            | Expr::BinOps { .. } => {
                self.write_char('(');
                self.write_expr_inner(expr);
                self.write_char(')');
            }

            // Block expressions in atomic position: parenthesized.
            // The closing `)` stays inline so subsequent application args
            // can follow at the right column.
            Expr::IfElse { .. }
            | Expr::CaseOf { .. }
            | Expr::LetIn { .. }
            | Expr::Lambda { .. } => {
                self.write_char('(');
                self.write_expr_inner(expr);
                self.write_char(')');
            }
        }
    }

    /// Write a comma-separated list of expressions with adaptive layout.
    /// Uses single-line when all elements are single-line, multi-line otherwise.
    fn write_comma_sep(&mut self, open: &str, close: &str, elems: &[Spanned<Expr>]) {
        let any_multiline = elems.iter().any(|e| is_multiline(&e.value));
        if any_multiline {
            // Multi-line: one element per indented line.
            self.write(open.trim_end());
            self.indent();
            for (i, elem) in elems.iter().enumerate() {
                self.newline_indent();
                if i == 0 {
                    self.write("  ");
                } else {
                    self.write(", ");
                }
                self.write_expr(&elem.value);
            }
            self.newline_indent();
            self.write(close.trim_start());
            self.dedent();
        } else {
            // Single-line: all on one line.
            self.write(open);
            for (i, elem) in elems.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write_expr(&elem.value);
            }
            self.write(close);
        }
    }

    fn write_record_setter(&mut self, setter: &RecordSetter) {
        self.write(&setter.field.value);
        self.write(" = ");
        self.write_expr(&setter.value.value);
    }

    fn write_if_expr(&mut self, branches: &[(Spanned<Expr>, Spanned<Expr>)], else_branch: &Expr) {
        // Single-line when all branches are simple non-block expressions.
        let all_simple = branches.len() == 1
            && branches
                .iter()
                .all(|(c, b)| !is_multiline(&c.value) && !is_multiline(&b.value))
            && !is_multiline(else_branch);

        if all_simple {
            let (cond, body) = &branches[0];
            self.write("if ");
            self.write_expr(&cond.value);
            self.write(" then ");
            self.write_expr(&body.value);
            self.write(" else ");
            self.write_expr(else_branch);
        } else {
            for (i, (cond, body)) in branches.iter().enumerate() {
                if i == 0 {
                    self.write("if ");
                } else {
                    self.write("else if ");
                }
                self.write_expr(&cond.value);
                self.write(" then");
                self.indent();
                self.newline_indent();
                self.write_expr(&body.value);
                self.dedent();
                self.newline();
                self.newline_indent();
            }
            self.write("else");
            self.indent();
            self.newline_indent();
            self.write_expr(else_branch);
            self.dedent();
        }
    }

    fn write_case_expr(&mut self, subject: &Expr, branches: &[CaseBranch]) {
        self.write("case ");
        self.write_expr(subject);
        self.write(" of");
        self.indent();
        for branch in branches {
            self.newline_indent();
            // Emit leading comments on the branch pattern.
            self.write_leading_comments(&branch.pattern.comments);
            self.write_pattern(&branch.pattern.value);
            self.write(" ->");
            self.indent();
            self.newline_indent();
            // Emit leading comments on the branch body.
            self.write_leading_comments(&branch.body.comments);
            self.write_expr(&branch.body.value);
            self.dedent();
        }
        self.dedent();
    }

    fn write_let_expr(&mut self, declarations: &[Spanned<LetDeclaration>], body: &Expr) {
        self.write("let");
        self.indent();
        for decl in declarations {
            self.newline_indent();
            // Emit leading comments on this let declaration.
            self.write_leading_comments(&decl.comments);
            self.write_let_declaration(&decl.value);
        }
        self.dedent();
        self.newline_indent();
        self.write("in");
        self.newline_indent();
        self.write_expr(body);
    }

    fn write_let_declaration(&mut self, decl: &LetDeclaration) {
        match decl {
            LetDeclaration::Function(func) => {
                if let Some(sig) = &func.signature {
                    self.write_signature(&sig.value);
                    self.newline_indent();
                }
                self.write_function_impl(&func.declaration.value);
            }
            LetDeclaration::Destructuring { pattern, body } => {
                self.write_pattern(&pattern.value);
                self.write(" =");
                self.indent();
                self.newline_indent();
                self.write_expr(&body.value);
                self.dedent();
            }
        }
    }

    fn write_lambda(&mut self, args: &[Spanned<Pattern>], body: &Expr) {
        self.write("\\");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.write_char(' ');
            }
            self.write_pattern_atomic(&arg.value);
        }
        self.write(" -> ");
        self.write_expr(body);
    }

    // ── Literals ─────────────────────────────────────────────────────

    fn write_literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Char(c) => {
                self.write_char('\'');
                self.write_escaped_char(*c);
                self.write_char('\'');
            }
            Literal::String(s) => {
                self.write_char('"');
                self.write_escaped_string(s);
                self.write_char('"');
            }
            Literal::MultilineString(s) => {
                self.write("\"\"\"");
                self.write(s);
                self.write("\"\"\"");
            }
            Literal::Int(n) => self.write(&n.to_string()),
            Literal::Hex(n) => {
                self.write("0x");
                self.write(&format!("{n:02X}"));
            }
            Literal::Float(f) => {
                let s = f.to_string();
                self.write(&s);
                if !s.contains('.') {
                    self.write(".0");
                }
            }
        }
    }

    fn write_escaped_char(&mut self, c: char) {
        match c {
            '\n' => self.write("\\n"),
            '\r' => self.write("\\r"),
            '\t' => self.write("\\t"),
            '\\' => self.write("\\\\"),
            '\'' => self.write("\\'"),
            '"' => self.write("\\\""),
            c if c.is_control() => {
                self.write(&format!("\\u{{{:04X}}}", c as u32));
            }
            c => self.write_char(c),
        }
    }

    fn write_escaped_string(&mut self, s: &str) {
        for c in s.chars() {
            self.write_escaped_char(c);
        }
    }
}

// ── Multiline detection ──────────────────────────────────────────────
//
// Inspired by elm-format's `allSingles` / `Box` model: eagerly determine
// whether an expression would produce multi-line output. Block expressions
// (case/if/let/lambda) are always multi-line. Containers are multi-line
// if any child is multi-line.

fn is_multiline(expr: &Expr) -> bool {
    match expr {
        // Block expressions are always multi-line.
        Expr::IfElse {
            branches,
            else_branch,
            ..
        } => {
            // Single-line if: only when simple condition + simple branches
            if branches.len() == 1 {
                let (c, b) = &branches[0];
                if !is_multiline(&c.value)
                    && !is_multiline(&b.value)
                    && !is_multiline(&else_branch.value)
                {
                    return false;
                }
            }
            true
        }
        Expr::CaseOf { .. } | Expr::LetIn { .. } => true,
        Expr::Lambda { body, .. } => is_multiline(&body.value),

        // Application: args are always inline (block args get parenthesized).
        // An application is multi-line only if it contains a multi-line
        // parenthesized block, which will have the closing ) on its own line.
        Expr::Application(args) => args.iter().any(|a| is_multiline(&a.value)),
        Expr::List(elems) => elems.iter().any(|e| is_multiline(&e.value)),
        Expr::Tuple(elems) => elems.iter().any(|e| is_multiline(&e.value)),
        Expr::Record(fields) => fields.iter().any(|f| is_multiline(&f.value.value.value)),
        Expr::RecordUpdate { updates, .. } => {
            updates.iter().any(|f| is_multiline(&f.value.value.value))
        }

        // Operators: multi-line if either side is multi-line.
        Expr::OperatorApplication { left, right, .. } => {
            is_multiline(&left.value) || is_multiline(&right.value)
        }

        // Wrapping: inherit from inner.
        Expr::Parenthesized(inner) => is_multiline(&inner.value),
        Expr::Negation(inner) => is_multiline(&inner.value),

        // Everything else is single-line.
        _ => false,
    }
}

// ── Standalone helpers ───────────────────────────────────────────────

fn op_precedence(op: &str) -> u8 {
    match op {
        "<|" | "|>" => 0,
        "||" => 2,
        "&&" => 3,
        "==" | "/=" | "<" | ">" | "<=" | ">=" => 4,
        "::" | "++" => 5,
        "+" | "-" => 6,
        "*" | "/" | "//" => 7,
        "^" => 8,
        "<<" | ">>" => 9,
        _ => 9,
    }
}

fn is_right_assoc(op: &str) -> bool {
    matches!(op, "<|" | "||" | "&&" | "::" | "++" | "^" | ">>")
}

/// Convenience function: print an `ElmModule` to a string with default config.
pub fn print(module: &ElmModule) -> String {
    Printer::new(PrintConfig::default()).print_module(module)
}
