extern crate proc_macro;

use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::{quote, ToTokens};
use syn;

const CRATE_NAME: &str = "specs_dsl";

#[proc_macro_attribute]
pub fn data_item(_attrs: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_data_item(item.into()).into()
}

#[proc_macro_attribute]
pub fn data_view(attrs: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_data_view(attrs.into(), item.into()).into()
}

#[proc_macro_attribute]
pub fn system(attrs: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_system(attrs.into(), item.into()).into()
}

trait AttributeUtils {
    fn is_name(&self, name: &str) -> bool;
}

impl AttributeUtils for syn::Attribute {
    fn is_name(&self, name: &str) -> bool {
        self.path.get_ident().map(|ident| ident == name).unwrap_or_default()
    }
}

fn expand_data_item(input: TokenStream) -> TokenStream {
    let mut item = parse_struct(input);

    let crate_name = crate_name();
    let Lifetimes {
        item_lifetime,
        ext_lifetime,
        ext_generics,
    } = get_lifetimes(&item);

    let item_tuple = syn::TypeTuple {
        paren_token: Default::default(),
        elems: item.fields.iter().map(|field| field.ty.clone()).collect(),
    };
    let item_init_from_tuple_fields: Vec<_> = item
        .fields
        .iter()
        .enumerate()
        .map(|(i, syn::Field { ident, .. })| {
            let i = Literal::usize_unsuffixed(i);
            quote! { #ident: t.#i }
        })
        .collect();
    let fields = extract_field_data(&mut item);

    let system_data_attr = extract_attr(&mut item.attrs, "system_data");
    let vis = &item.vis;
    let system_data_defs = system_data_attr.map(|attr| {
        let type_name = attr.parse_args::<syn::Ident>().expect("Cannot parse system data name");
        let lifetime = syn::Lifetime::new("'a", Span::call_site());
        let storages = storages(&lifetime, None, &fields);
        let view_store_lifetime = syn::Lifetime::new("'b", Span::call_site());
        let MainViews {
            view_type,
            view_ret,
            view_mut_type,
            view_mut_ret
        } = storages_main_views(&view_store_lifetime, &lifetime, &fields);
        let main_views_trait_name = syn::Ident::new(&format!("{}MainView", type_name), Span::call_site());

        quote! {
            #vis type #type_name<#lifetime> = #storages;

            pub trait #main_views_trait_name<'a> {
                type ViewAllImmutable;
                type ViewAllWithMut;

                fn view(&'a self) -> Self::ViewAllImmutable;
                fn view_mut(&'a mut self) -> Self::ViewAllWithMut;
            }

            impl<#lifetime, #view_store_lifetime: #lifetime> #main_views_trait_name<#lifetime> for #type_name<#view_store_lifetime> {
                type ViewAllImmutable = #view_type;
                type ViewAllWithMut = #view_mut_type;

                fn view(&#lifetime self) -> Self::ViewAllImmutable {
                    #view_ret
                }

                fn view_mut(&#lifetime mut self) -> Self::ViewAllWithMut {
                    #view_mut_ret
                }
            }
        }
    });

    let (impl_data_view_generics, _, _) = ext_generics.split_for_impl();
    let storages_ref = storages(&ext_lifetime, Some(&item_lifetime), &fields);
    let (impl_generics, type_generics, where_clause) = item.generics.split_for_impl();
    let item_type_name = &item.ident;

    quote! {
        #item

        impl#impl_generics From<#item_tuple> for #item_type_name#type_generics #where_clause {
            fn from(t: #item_tuple) -> Self {
                Self {
                    #(#item_init_from_tuple_fields),*
                }
            }
        }

        impl#impl_data_view_generics #crate_name::DataItem<#item_lifetime, #ext_lifetime> for #item_type_name#type_generics #where_clause {
            type View = #storages_ref;
        }

        #system_data_defs
    }
}

fn expand_data_view(_attrs: TokenStream, _input: TokenStream) -> TokenStream {
    unimplemented!();
}

fn expand_system(attrs: TokenStream, input: TokenStream) -> TokenStream {
    let system_data = syn::parse2::<syn::Ident>(attrs).expect("Failed parse attribute parameter");
    let mut item = syn::parse2::<syn::ItemImpl>(input).expect("Failed parse system impl block");

    let crate_name = crate_name();
    let system_type = (*item.self_ty).clone();
    let run_method = item
        .items
        .iter_mut()
        .find_map(|item| match item {
            syn::ImplItem::Method(method) => {
                if extract_attr(&mut method.attrs, "run").is_some() {
                    Some(method.sig.ident.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .expect("Cannot find the run-annotated method");

    quote! {
        #item

        impl<'a> #crate_name::specs::System<'a> for #system_type {
            type SystemData = #system_data<'a>;

            fn run(&mut self, data: Self::SystemData) {
                self.#run_method(data);
            }
        }
    }
}

fn parse_struct(input: TokenStream) -> syn::ItemStruct {
    let item = syn::parse2(input).expect("Failed parse input token stream");
    match item {
        syn::Item::Struct(item) => item,
        _ => panic!("The data item must be a struct"),
    }
}

struct Lifetimes {
    item_lifetime: syn::Lifetime,
    ext_lifetime: syn::Lifetime,
    ext_generics: syn::Generics,
}

fn get_lifetimes(item: &syn::ItemStruct) -> Lifetimes {
    let item_lifetime = match item
        .generics
        .params
        .iter()
        .next()
        .expect("The data item must have at least one generic parameter with lifetime")
    {
        syn::GenericParam::Lifetime(def) => def.lifetime.clone(),
        _ => panic!("The data item must have one lifetime parameter"),
    };

    let ext_lifetime = syn::Lifetime::new(&format!("'b{}", item_lifetime.ident), Span::call_site());
    let mut ext_lifetime_def = syn::LifetimeDef::new(ext_lifetime.clone());
    ext_lifetime_def.bounds.push(item_lifetime.clone());

    let mut ext_params = vec![];
    let mut is_lifetime_group_processed = false;
    for param in &item.generics.params {
        if let syn::GenericParam::Lifetime(_) = param {
        } else {
            ext_params.push(syn::GenericParam::Lifetime(ext_lifetime_def.clone()));
            is_lifetime_group_processed = true;
        }
        ext_params.push(param.clone());
    }
    if !is_lifetime_group_processed {
        ext_params.push(syn::GenericParam::Lifetime(ext_lifetime_def));
    }
    let mut ext_generics = item.generics.clone();
    ext_generics.params = ext_params.into_iter().collect();

    Lifetimes {
        item_lifetime,
        ext_lifetime,
        ext_generics,
    }
}

fn get_attr<'a>(attrs: &'a [syn::Attribute], name: &str) -> Option<&'a syn::Attribute> {
    attrs.iter().find(|attr| attr.is_name(name))
}

fn extract_attr(attrs: &mut Vec<syn::Attribute>, name: &str) -> Option<syn::Attribute> {
    attrs
        .iter()
        .enumerate()
        .find_map(|(i, attr)| if attr.is_name(name) { Some(i) } else { None })
        .map(|idx| attrs.remove(idx))
}

fn crate_name() -> Ident {
    Ident::new(CRATE_NAME, Span::call_site())
}

enum ItemFieldKind {
    Entity,
    Component,
    Resource,
    MutComponent,
    MutResource,
}

impl ItemFieldKind {
    fn is_mut(&self) -> bool {
        match self {
            ItemFieldKind::MutComponent | ItemFieldKind::MutResource => true,
            _ => false,
        }
    }
}

struct ItemFieldData {
    kind: ItemFieldKind,
    field_type: syn::Type,
}

fn extract_field_data(item: &mut syn::ItemStruct) -> Vec<ItemFieldData> {
    item.fields
        .iter_mut()
        .map(|field| {
            let (is_mut, field_type) = match &field.ty {
                syn::Type::Reference(ref_type) => (ref_type.mutability.is_some(), (*ref_type.elem).clone()),
                _ => (false, field.ty.clone()),
            };
            let is_entity = extract_attr(&mut field.attrs, "entity").is_some()
                || format!("{}", field.ty.to_token_stream()).as_str() == "Entity";
            let is_resource = extract_attr(&mut field.attrs, "resource").is_some();
            let is_component = extract_attr(&mut field.attrs, "component").is_some() || (!is_entity && !is_resource);

            let kind = if is_component {
                if is_mut {
                    ItemFieldKind::MutComponent
                } else {
                    ItemFieldKind::Component
                }
            } else if is_resource {
                if is_mut {
                    ItemFieldKind::MutResource
                } else {
                    ItemFieldKind::Resource
                }
            } else if is_entity {
                ItemFieldKind::Entity
            } else {
                unreachable!("Unsupported item kind")
            };

            ItemFieldData {
                kind,
                field_type,
            }
        })
        .collect()
}

fn storages(
    store_lifetime: &syn::Lifetime,
    refs_lifetime: Option<&syn::Lifetime>,
    fields: &[ItemFieldData],
) -> TokenStream {
    let crate_name = crate_name();
    let storages: Vec<_> = fields
        .iter()
        .map(|field| {
            let ref_part = refs_lifetime
                .map(|lifetime| quote! { &#lifetime })
                .unwrap_or_else(|| quote! {});
            let ref_mut_part = refs_lifetime
                .map(|lifetime| quote! { &#lifetime mut })
                .unwrap_or_else(|| quote! {});
            let field_type = &field.field_type;

            match field.kind {
                ItemFieldKind::Entity => quote! { #ref_part #crate_name::specs::Entities<#store_lifetime> },
                ItemFieldKind::Component => {
                    quote! { #ref_part #crate_name::specs::ReadStorage<#store_lifetime, #field_type> }
                }
                ItemFieldKind::Resource => quote! { #ref_part #crate_name::specs::Read<#store_lifetime, #field_type> },
                ItemFieldKind::MutComponent => {
                    quote! { #ref_mut_part #crate_name::specs::WriteStorage<#store_lifetime, #field_type> }
                }
                ItemFieldKind::MutResource => {
                    quote! { #ref_mut_part #crate_name::specs::Write<#store_lifetime, #field_type> }
                }
            }
        })
        .collect();

    if storages.len() == 1 {
        storages.into_iter().next().unwrap()
    } else {
        quote! { (#(#storages),*) }
    }
}

struct MainViews {
    view_type: TokenStream,
    view_ret: TokenStream,
    view_mut_type: TokenStream,
    view_mut_ret: TokenStream,
}

fn storages_main_views(
    store_lifetime: &syn::Lifetime,
    refs_lifetime: &syn::Lifetime,
    fields: &[ItemFieldData],
) -> MainViews {
    let crate_name = crate_name();
    let mut view_indexes = vec![];
    let view_storages: Vec<_> = fields
        .iter()
        .enumerate()
        .filter_map(|(idx, field)| {
            let field_type = &field.field_type;

            let store = match field.kind {
                ItemFieldKind::Entity => Some(quote! { &#refs_lifetime #crate_name::specs::Entities<#store_lifetime> }),
                ItemFieldKind::Component => {
                    Some(quote! { &#refs_lifetime #crate_name::specs::ReadStorage<#store_lifetime, #field_type> })
                }
                ItemFieldKind::Resource => {
                    Some(quote! { &#refs_lifetime #crate_name::specs::Read<#store_lifetime, #field_type> })
                }
                _ => None,
            };

            if store.is_some() {
                view_indexes.push(idx);
            }
            store
        })
        .collect();

    let (view_type, view_ret) = if view_storages.is_empty() {
        (quote! { () }, quote! { () })
    } else if view_storages.len() == 1 {
        let idx = Literal::usize_unsuffixed(view_indexes[0]);
        (view_storages.into_iter().next().unwrap(), quote! { &self.#idx })
    } else {
        let refs = view_indexes.iter().map(|&idx| {
            let idx = Literal::usize_unsuffixed(idx);
            quote! { &self.#idx }
        });
        (quote! { (#(#view_storages),*) }, quote! { (#(#refs),*) })
    };

    let (view_mut_type, view_mut_ret) = if view_indexes.len() == fields.len() {
        (quote! { () }, quote! { () })
    } else {
        let ret_tuple_fields: Vec<_> = fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let i = Literal::usize_unsuffixed(i);
                if field.kind.is_mut() {
                    quote! { &mut self.#i }
                } else {
                    quote! { &self.#i }
                }
            })
            .collect();

        let ret = if ret_tuple_fields.len() == 1 {
            ret_tuple_fields.into_iter().next().unwrap()
        } else {
            quote! { (#(#ret_tuple_fields),*) }
        };

        (storages(store_lifetime, Some(refs_lifetime), fields), ret)
    };

    MainViews {
        view_type,
        view_ret,
        view_mut_type,
        view_mut_ret,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_data_item() {
        let item = quote! {
            #[system_data(PosVelSystemData)]
            #[derive(Clone, Copy)]
            struct PosVel<'a> {
                pos: &'a mut Pos,
                vel: &'a Vel,
            }
        };
        let output = expand_data_item(item).to_string();

        #[rustfmt::skip]
        assert_eq!(output, "\
# [ derive ( Clone , Copy ) ] \
struct PosVel < 'a > { \
pos : & 'a mut Pos , \
vel : & 'a Vel , \
} \
impl < 'a > From < ( & 'a mut Pos , & 'a Vel ) > for PosVel < 'a > { \
fn from ( t : ( & 'a mut Pos , & 'a Vel ) ) -> Self { \
Self { pos : t . 0 , vel : t . 1 } \
} \
} \
impl < 'a , 'ba : 'a > specs_dsl :: DataItem < 'a , 'ba > for PosVel < 'a > { \
type View = ( & 'a mut specs_dsl :: specs :: WriteStorage < 'ba , Pos > , & 'a specs_dsl :: specs :: ReadStorage < 'ba , Vel > ) ; \
} \
type PosVelSystemData < 'a > = ( specs_dsl :: specs :: WriteStorage < 'a , Pos > , specs_dsl :: specs :: ReadStorage < 'a , Vel > ) ; \
pub trait PosVelSystemDataMainView < 'a > { \
type ViewAllImmutable ; \
type ViewAllWithMut ; \
fn view ( & 'a self ) -> Self :: ViewAllImmutable ; \
fn view_mut ( & 'a mut self ) -> Self :: ViewAllWithMut ; \
} \
impl < 'a , 'b : 'a > PosVelSystemDataMainView < 'a > for PosVelSystemData < 'b > { \
type ViewAllImmutable = & 'a specs_dsl :: specs :: ReadStorage < 'b , Vel > ; \
type ViewAllWithMut = ( & 'a mut specs_dsl :: specs :: WriteStorage < 'b , Pos > , & 'a specs_dsl :: specs :: ReadStorage < 'b , Vel > ) ; \
fn view ( & 'a self ) -> Self :: ViewAllImmutable { \
& self . 1 \
} \
fn view_mut ( & 'a mut self ) -> Self :: ViewAllWithMut { \
( & mut self . 0 , & self . 1 ) \
} \
}");
    }

    #[test]
    fn test_expand_system() {
        let attrs = quote! { Test };
        let item = quote! {
            impl PhysicsSystem {
                #[run]
                fn change_pos(&mut self, mut data: SystemDataType<Self>) {
                    unimplemented!()
                }
            }
        };
        let output = expand_system(attrs, item).to_string();

        #[rustfmt::skip]
        assert_eq!(output, "\
impl PhysicsSystem { \
fn change_pos ( & mut self , mut data : SystemDataType < Self > ) { \
unimplemented ! ( ) \
} \
} \
impl < 'a > specs_dsl :: specs :: System < 'a > for PhysicsSystem { \
type SystemData = Test < 'a > ; \
fn run ( & mut self , data : Self :: SystemData ) { \
self . change_pos ( data ) ; \
} \
}");
    }
}
