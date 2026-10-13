//! External tool registry: PATH detection, argument templates, and built-in plus custom tools.

pub mod registry;
pub mod template;
pub mod which;

pub use registry::{
    Activity, DetectedProgram, OutputParser, ToolDefinition, ToolEntry, ToolRegistry,
};
pub use template::{render_args, validate_template};
pub use which::find_on_path;
