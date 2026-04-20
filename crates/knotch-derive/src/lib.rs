//! Procedural macros for knotch.
//!
//! - `#[derive(PhaseKind)]` — emits `knotch_kernel::PhaseKind` impls for enums whose
//!   variants represent ordered phases.
//! - `#[derive(MilestoneKind)]` — emits `MilestoneKind` for simple unit-variant enums or
//!   newtype wrappers.
//! - `#[derive(GateKind)]` — same shape as milestone.
//! - `#[derive(Sensitive)]` — marker trait for PII redaction.
//! - `#[workflow(name = …, phase = …, milestone = …, gate = …)]` — attribute macro on a
//!   marker struct that emits the full `WorkflowKind` impl.
//!
//! Ergonomics prioritize the common case: unit-variant enums whose
//! id is the kebab-case of the variant name. Attribute
//! customizations can follow in a later phase.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Data, DataEnum, DeriveInput, Fields, ItemStruct, LitInt, LitStr, Path, Token,
    parse::{Parse, ParseStream, Parser},
    parse_macro_input,
    punctuated::Punctuated,
};

/// `#[derive(PhaseKind)]` — requires an enum with only unit variants.
/// Variant declaration order determines phase order; the trailing
/// variant's `next()` returns `None`.
#[proc_macro_derive(PhaseKind)]
pub fn derive_phase_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_phase_kind(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_phase_kind(input: &DeriveInput) -> Result<TokenStream2, syn::Error> {
    let enum_data = expect_unit_enum(input, "PhaseKind")?;
    let name = &input.ident;
    let variants: Vec<_> = enum_data.variants.iter().map(|v| &v.ident).collect();
    let ids: Vec<String> = variants.iter().map(|v| to_kebab(&v.to_string())).collect();
    let id_arms = variants.iter().zip(&ids).map(|(variant, id)| {
        quote! { #name::#variant => #id, }
    });

    Ok(quote! {
        #[automatically_derived]
        impl ::knotch_kernel::PhaseKind for #name {
            fn id(&self) -> ::std::borrow::Cow<'_, str> {
                ::std::borrow::Cow::Borrowed(match self {
                    #(#id_arms)*
                })
            }

            fn is_skippable(&self, _reason: &::knotch_kernel::event::SkipKind) -> bool {
                false
            }
        }
    })
}

/// `#[derive(MilestoneKind)]` — requires an enum with unit variants
/// OR a newtype tuple struct carrying a single `CompactString`.
#[proc_macro_derive(MilestoneKind)]
pub fn derive_milestone_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_identity_kind(&input, quote! { ::knotch_kernel::MilestoneKind }) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// `#[derive(GateKind)]` — same shape as MilestoneKind.
#[proc_macro_derive(GateKind)]
pub fn derive_gate_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_identity_kind(&input, quote! { ::knotch_kernel::GateKind }) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_identity_kind(
    input: &DeriveInput,
    trait_path: TokenStream2,
) -> Result<TokenStream2, syn::Error> {
    let name = &input.ident;
    let body = match &input.data {
        Data::Enum(enum_data) => {
            require_unit_variants(enum_data, "MilestoneKind / GateKind")?;
            let arms = enum_data.variants.iter().map(|v| {
                let vident = &v.ident;
                let id = to_kebab(&vident.to_string());
                quote! { #name::#vident => ::std::borrow::Cow::Borrowed(#id), }
            });
            quote! {
                match self {
                    #(#arms)*
                }
            }
        }
        Data::Struct(s) => match &s.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                quote! { ::std::borrow::Cow::Borrowed(self.0.as_ref()) }
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "MilestoneKind/GateKind on a struct requires a newtype with a single field",
                ));
            }
        },
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(input, "unions are not supported"));
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl #trait_path for #name {
            fn id(&self) -> ::std::borrow::Cow<'_, str> {
                #body
            }
        }
    })
}

/// `#[derive(Sensitive)]` — marks a type for tracing redaction.
#[proc_macro_derive(Sensitive)]
pub fn derive_sensitive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    quote! {
        #[automatically_derived]
        impl ::knotch_kernel::causation::Sensitive for #name {}
    }
    .into()
}

fn expect_unit_enum<'a>(
    input: &'a DeriveInput,
    trait_name: &str,
) -> Result<&'a DataEnum, syn::Error> {
    match &input.data {
        Data::Enum(e) => {
            require_unit_variants(e, trait_name)?;
            Ok(e)
        }
        _ => Err(syn::Error::new_spanned(input, format!("`{trait_name}` derive requires an enum"))),
    }
}

fn require_unit_variants(enum_data: &DataEnum, trait_name: &str) -> Result<(), syn::Error> {
    if enum_data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &enum_data.variants,
            format!("`{trait_name}` derive requires at least one variant"),
        ));
    }
    for variant in &enum_data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                format!("`{trait_name}` derive requires unit variants only"),
            ));
        }
    }
    Ok(())
}

fn to_kebab(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, ch) in ident.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('-');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------
// #[workflow] attribute macro
// ---------------------------------------------------------------------

/// Parsed arguments for `#[workflow(...)]`.
struct WorkflowArgs {
    name: LitStr,
    schema_version: Option<LitInt>,
    phase: Path,
    milestone: Path,
    gate: Path,
    extension: Option<Path>,
    required_phases: Option<Path>,
}

impl Parse for WorkflowArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let pairs: Punctuated<syn::MetaNameValue, Token![,]> = Punctuated::parse_terminated(input)?;
        let mut name = None;
        let mut schema_version = None;
        let mut phase = None;
        let mut milestone = None;
        let mut gate = None;
        let mut extension = None;
        let mut required_phases = None;
        for pair in pairs {
            let ident = pair
                .path
                .get_ident()
                .ok_or_else(|| syn::Error::new_spanned(&pair.path, "expected identifier key"))?;
            let key = ident.to_string();
            match key.as_str() {
                "name" => name = Some(syn::parse2::<LitStr>(pair.value.to_token_stream())?),
                "schema_version" => {
                    schema_version = Some(syn::parse2::<LitInt>(pair.value.to_token_stream())?)
                }
                "phase" => phase = Some(syn::parse2::<Path>(pair.value.to_token_stream())?),
                "milestone" => milestone = Some(syn::parse2::<Path>(pair.value.to_token_stream())?),
                "gate" => gate = Some(syn::parse2::<Path>(pair.value.to_token_stream())?),
                "extension" => extension = Some(syn::parse2::<Path>(pair.value.to_token_stream())?),
                "required_phases" => {
                    required_phases = Some(syn::parse2::<Path>(pair.value.to_token_stream())?)
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        &pair.path,
                        format!(
                            "unknown #[workflow] argument `{other}` — \
                             expected one of: name, schema_version, phase, \
                             milestone, gate, extension, required_phases"
                        ),
                    ));
                }
            }
        }
        Ok(Self {
            name: name.ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "#[workflow] requires `name = \"...\"`",
                )
            })?,
            schema_version,
            phase: phase.ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "#[workflow] requires `phase = <PhaseType>`",
                )
            })?,
            milestone: milestone.ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "#[workflow] requires `milestone = <MilestoneType>`",
                )
            })?,
            gate: gate.ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "#[workflow] requires `gate = <GateType>`",
                )
            })?,
            extension,
            required_phases,
        })
    }
}

use quote::ToTokens as _;

/// `#[workflow(name = "…", phase = …, milestone = …, gate = …)]` —
/// attribute macro that emits `impl knotch_kernel::WorkflowKind for
/// <Marker>` on a zero-sized marker struct.
///
/// Mandatory keys: `name`, `phase`, `milestone`, `gate`.
/// Optional: `schema_version` (default 1), `extension` (default `()`),
/// `required_phases` (a `fn(&Scope) -> &'static [Phase]` path).
///
/// ```ignore
/// #[workflow(name = "myflow", phase = MyPhase,
///            milestone = MyMilestone, gate = MyGate)]
/// pub struct MyFlow;
/// ```
#[proc_macro_attribute]
pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args: WorkflowArgs = match WorkflowArgs::parse.parse(attr) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };
    let item = parse_macro_input!(item as ItemStruct);
    match expand_workflow(&args, &item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_workflow(args: &WorkflowArgs, item: &ItemStruct) -> Result<TokenStream2, syn::Error> {
    let marker = &item.ident;
    let name_lit = &args.name;
    let schema = match &args.schema_version {
        Some(lit) => quote! { #lit },
        None => quote! { 1u32 },
    };
    let phase = &args.phase;
    let milestone = &args.milestone;
    let gate = &args.gate;
    let extension = match &args.extension {
        Some(p) => quote! { #p },
        None => quote! { () },
    };
    let required = match &args.required_phases {
        Some(p) => quote! { #p(scope) },
        None => quote! {
            ::core::compile_error!(
                "#[workflow] requires `required_phases = path::to::fn` \
                 unless you impl `WorkflowKind::required_phases` manually"
            )
        },
    };

    Ok(quote! {
        #item

        #[automatically_derived]
        impl ::knotch_kernel::WorkflowKind for #marker {
            type Phase = #phase;
            type Milestone = #milestone;
            type Gate = #gate;
            type Extension = #extension;
            fn name(&self) -> ::std::borrow::Cow<'_, str> {
                ::std::borrow::Cow::Borrowed(#name_lit)
            }
            fn schema_version(&self) -> u32 {
                #schema
            }
            fn required_phases(&self, scope: &::knotch_kernel::Scope)
                -> ::std::borrow::Cow<'_, [Self::Phase]>
            {
                ::std::borrow::Cow::Borrowed(#required)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::to_kebab;

    #[test]
    fn kebab_converts_pascal() {
        assert_eq!(to_kebab("Specify"), "specify");
        assert_eq!(to_kebab("InReview"), "in-review");
        assert_eq!(to_kebab("G0"), "g0");
    }
}
