//! A checked program kept alive for the compiler API's queries.
//!
//! The one-shot check tears its state down when it finishes. A
//! [`RetainedProgram`] keeps it: queries read the checker's own results (see
//! [`index`]) and resolve types with the checker's own resolution, under the
//! engine lock and with the program's state installed (see [`environment`]).

mod environment;
mod index;
mod query;
mod retained;

pub(crate) use environment::{claim_thread_caches, engine_lock, next_program_id};
pub(crate) use index::{
    capture_file_scope, record_const_operand_type, record_contextual_signature, record_instantiated_arguments, record_contextual_type, record_declaration_type, record_expression_type, record_literal_type,
    record_local_type_scope, record_own_function_type, with_inferred_predicate, record_parameter_type, record_span_type, recording,
};
pub use index::FileSemanticIndex;
pub use query::{PropertyInfo, Query, settled_read};
pub use retained::RetainedProgram;
