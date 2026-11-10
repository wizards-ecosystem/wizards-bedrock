use syn::Macro;
use syn::visit::{self, Visit};

use crate::paths::Paths;

use super::{asm_tokens, rust_sources_below};

/// The raw port-claim doors, which take a numeric port rather than a device.
const RAW_PORT_CLAIMS: [&str; 4] = ["claim", "claim_pair", "claim_block", "claim_block_pair"];

#[derive(Default)]
struct RawPortClaims {
    calls: Vec<String>,
}

impl<'ast> Visit<'ast> for RawPortClaims {
    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let mut tokens = Vec::new();
        asm_tokens(invocation.tokens.clone(), &mut tokens);
        let through_port = tokens
            .iter()
            .any(|token| token.trim_start_matches("r#") == "port");
        if through_port {
            for token in &tokens {
                let door = token.trim_start_matches("r#");
                if RAW_PORT_CLAIMS.contains(&door) || door == "*" {
                    self.calls.push(format!("macro port::{door}"));
                }
            }
            for rename in tokens.windows(2) {
                if matches!(rename[0].trim_start_matches("r#"), "port" | "self")
                    && rename[1] == "as"
                {
                    self.calls.push("macro port as".to_string());
                }
            }
        }
        visit::visit_macro(self, invocation);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.iter().any(|segment| segment == "port")
            && segments
                .last()
                .is_some_and(|name| RAW_PORT_CLAIMS.contains(&name.as_str()))
        {
            self.calls.push(segments.join("::"));
        }
        visit::visit_expr_path(self, path);
    }

    // An import can name a door, or the module, by a path with no `port` segment.
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.imports(&mut Vec::new(), &item.tree);
        visit::visit_item_use(self, item);
    }
}

impl RawPortClaims {
    /// Record each import of a raw door, rename of `port` or glob from it.
    fn imports(&mut self, prefix: &mut Vec<String>, tree: &syn::UseTree) {
        let through_port = prefix.iter().any(|segment| segment == "port");
        let path = |last: &str| {
            prefix
                .iter()
                .map(String::as_str)
                .chain([last])
                .collect::<Vec<_>>()
                .join("::")
        };
        match tree {
            syn::UseTree::Path(next) => {
                prefix.push(next.ident.to_string());
                self.imports(prefix, &next.tree);
                prefix.pop();
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.imports(prefix, item);
                }
            }
            syn::UseTree::Name(name) => {
                if through_port && RAW_PORT_CLAIMS.contains(&name.ident.to_string().as_str()) {
                    self.calls
                        .push(format!("use {}", path(&name.ident.to_string())));
                }
            }
            syn::UseTree::Rename(rename) => {
                let imported = if rename.ident == "self" {
                    prefix.last().cloned().unwrap_or_default()
                } else {
                    rename.ident.to_string()
                };
                if imported == "port"
                    || (through_port && RAW_PORT_CLAIMS.contains(&imported.as_str()))
                {
                    self.calls.push(format!(
                        "use {} as {}",
                        path(&rename.ident.to_string()),
                        rename.rename
                    ));
                }
            }
            syn::UseTree::Glob(_) => {
                if through_port {
                    self.calls.push(format!("use {}", path("*")));
                }
            }
        }
    }
}

/// Production kernel code names a device, never a raw port number (D-351).
///
/// A driver asks `port::i8042()` or `port::com1()`, not `port::claim(0x60)`.
/// Only the self-test harness may probe the registry with a raw claim, so a
/// component cannot reserve hardware it does not own.
#[test]
fn production_kernel_code_claims_no_port_by_number() {
    let paths = Paths::discover().expect("workspace paths");
    let mut offenders = Vec::new();
    for path in rust_sources_below(&paths.root().join("kernel/src")) {
        let relative = path
            .strip_prefix(paths.root())
            .expect("source below the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        // The self-test harness alone probes exclusivity with raw claims.
        if relative == "kernel/src/selftest.rs" || relative.starts_with("kernel/src/selftest/") {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
        let mut raw = RawPortClaims::default();
        raw.visit_file(&syntax);
        for call in raw.calls {
            offenders.push(format!("{relative}: `{call}`"));
        }
    }
    assert!(
        offenders.is_empty(),
        "production kernel code claims a port by number, or imports a door out of this scan's sight, instead of naming a device:\n{}",
        offenders.join("\n")
    );

    // The scanner sees a raw claim wherever `port` and the door both appear.
    let fixture = "fn drive() {\n\
            let _ = wost::arch::current::port::claim::<u32>(0xF4);\n\
            let _ = port::claim_block(0x40);\n\
            let _ = port::com1();\n\
            let _ = port::claim_spans::<2>(&spans);\n\
        }\n";
    let syntax = syn::parse_file(fixture).expect("fixture parses");
    let mut raw = RawPortClaims::default();
    raw.visit_file(&syntax);
    assert_eq!(
        raw.calls,
        [
            "wost::arch::current::port::claim".to_string(),
            "port::claim_block".to_string(),
        ]
    );

    // An import that drops the `port` segment is refused where it is written.
    let imports = "use wost::arch::current::port::{self, Port, claim_spans};\n\
        use wost::arch::current::port::claim;\n\
        use wost::arch::current::port::{claim_block as block, com1};\n\
        use wost::arch::current::port as io;\n\
        use wost::arch::current::port::{self as raw};\n\
        fn drive() { use wost::arch::current::port::*; }\n";
    let syntax = syn::parse_file(imports).expect("import fixture parses");
    let mut raw = RawPortClaims::default();
    raw.visit_file(&syntax);
    assert_eq!(
        raw.calls,
        [
            "use wost::arch::current::port::claim",
            "use wost::arch::current::port::claim_block as block",
            "use wost::arch::current::port as io",
            "use wost::arch::current::port::self as raw",
            "use wost::arch::current::port::*",
        ]
    );
}

#[test]
fn raw_port_claims_inside_macros_are_refused() {
    for (source, door) in [
        (
            "fn drive() { info!(\"{:?}\", port::claim::<u8>(0x80)); }",
            "claim",
        ),
        (
            "fn drive() { assert!(port::claim_block(0x80).is_ok()); }",
            "claim_block",
        ),
        (
            "fn drive() { info!(\"{:?}\", nested!({ port::claim_pair::<u8>(0x80, 0x81) })); }",
            "claim_pair",
        ),
        (
            "macro_rules! drive { () => { wost::arch::current::port::claim_block_pair(0x80, 0x90) }; }",
            "claim_block_pair",
        ),
        (
            "fn drive() { check!(r#port::r#claim::<u8>(0x80)); }",
            "claim",
        ),
        (
            "macro_rules! drive { () => { use wost::arch::current::port::*; }; }",
            "*",
        ),
        (
            "macro_rules! drive { () => { use wost::arch::current::port::{com1, claim_block as block}; }; }",
            "claim_block",
        ),
    ] {
        let syntax = syn::parse_file(source).expect("macro fixture parses");
        let mut raw = RawPortClaims::default();
        raw.visit_file(&syntax);
        assert_eq!(raw.calls, [format!("macro port::{door}")], "{source}");
    }
    for source in [
        "macro_rules! door { () => { use wost::arch::current::port as io; }; }",
        "macro_rules! door { () => { use wost::arch::current::port::{self as io}; }; }",
    ] {
        let syntax = syn::parse_file(source).expect("macro alias fixture parses");
        let mut raw = RawPortClaims::default();
        raw.visit_file(&syntax);
        assert!(!raw.calls.is_empty(), "a macro hid a port alias: {source}");
    }
    let source = "fn drive() { info!(\"port::claim(0x80)\");\n\
        assert!(port::com1().is_ok()); info!(\"{:?}\", port::claim_spans::<2>(&spans)); }";
    let syntax = syn::parse_file(source).expect("literal and typed-door fixture parses");
    let mut raw = RawPortClaims::default();
    raw.visit_file(&syntax);
    assert!(raw.calls.is_empty(), "literal or allowed door was refused");
}
