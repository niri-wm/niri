use knuffel::errors::DecodeError;

use crate::LayoutPart;

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct Workspace {
    #[knuffel(argument)]
    pub name: WorkspaceName,
    #[knuffel(child, unwrap(argument))]
    pub open_on_output: Option<String>,
    #[knuffel(child)]
    pub layout: Option<WorkspaceLayoutPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceName(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceLayoutPart(pub LayoutPart);

impl<S: knuffel::traits::ErrorSpan> knuffel::Decode<S> for WorkspaceLayoutPart {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        for child in node.children() {
            let name = &**child.node_name;

            // Check for disallowed properties.
            //
            // - empty-workspace-above-first is a monitor-level concept.
            // - insert-hint customization could make sense for workspaces, however currently it is
            //   also handled at the monitor level (since insert hints in-between workspaces are a
            //   monitor-level concept), so for now this config option would do nothing.
            if matches!(name, "empty-workspace-above-first" | "insert-hint") {
                ctx.emit_error(DecodeError::unexpected(
                    child,
                    "node",
                    format!("node `{name}` is not allowed inside `workspace.layout`"),
                ));
            }
        }

        LayoutPart::decode_node(node, ctx).map(Self)
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::DecodeScalar<S> for WorkspaceName {
    fn type_check(
        type_name: &Option<knuffel::span::Spanned<knuffel::ast::TypeName, S>>,
        ctx: &mut knuffel::decode::Context<S>,
    ) {
        if let Some(type_name) = &type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }
    }

    fn raw_decode(
        val: &knuffel::span::Spanned<knuffel::ast::Literal, S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<WorkspaceName, DecodeError<S>> {
        #[derive(Debug)]
        struct WorkspaceNameSet(Vec<String>);
        match &**val {
            knuffel::ast::Literal::String(ref s) => {
                let mut name_set: Vec<String> = match ctx.get::<WorkspaceNameSet>() {
                    Some(h) => h.0.clone(),
                    None => Vec::new(),
                };

                if name_set.iter().any(|name| name.eq_ignore_ascii_case(s)) {
                    ctx.emit_error(DecodeError::unexpected(
                        val,
                        "named workspace",
                        format!("duplicate named workspace: {s}"),
                    ));
                    return Ok(Self(String::new()));
                }

                name_set.push(s.to_string());
                ctx.set(WorkspaceNameSet(name_set));
                Ok(Self(s.clone().into()))
            }
            _ => {
                ctx.emit_error(DecodeError::unsupported(
                    val,
                    "workspace names must be strings",
                ));
                Ok(Self(String::new()))
            }
        }
    }
}
