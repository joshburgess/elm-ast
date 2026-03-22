pub mod no_always_identity;
pub mod no_boolean_case;
pub mod no_debug;
pub mod no_empty_let;
pub mod no_empty_record_update;
pub mod no_if_true_false;
pub mod no_missing_type_annotation;
pub mod no_nested_negation;
pub mod no_redundant_cons;
pub mod no_single_pattern_case;
pub mod no_unnecessary_parens;
pub mod no_unused_imports;
pub mod no_unused_variables;
pub mod no_wildcard_pattern_last;

use crate::rule::Rule;

/// Return all built-in rules.
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(no_unused_imports::NoUnusedImports),
        Box::new(no_unused_variables::NoUnusedVariables),
        Box::new(no_debug::NoDebug),
        Box::new(no_missing_type_annotation::NoMissingTypeAnnotation),
        Box::new(no_single_pattern_case::NoSinglePatternCase),
        Box::new(no_boolean_case::NoBooleanCase),
        Box::new(no_if_true_false::NoIfTrueFalse),
        Box::new(no_unnecessary_parens::NoUnnecessaryParens),
        Box::new(no_nested_negation::NoNestedNegation),
        Box::new(no_empty_let::NoEmptyLet),
        Box::new(no_empty_record_update::NoEmptyRecordUpdate),
        Box::new(no_always_identity::NoAlwaysIdentity),
        Box::new(no_redundant_cons::NoRedundantCons),
        Box::new(no_wildcard_pattern_last::NoWildcardPatternLast),
    ]
}
