extern crate proc_macro;

use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::quote;
use syn;
use syn::{Attribute, Item, Type};

const CRATE_NAME: &str = "specs_dsl";

#[proc_macro_attribute]
pub fn data_item(_attrs: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_data_item(item.into()).into()
}

#[proc_macro_attribute]
pub fn data_view(attrs: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_data_view(attrs.into(), item.into()).into()
}

trait AttributeUtils {
    fn is_name(&self, name: &str) -> bool;
}

impl AttributeUtils for Attribute {
    fn is_name(&self, name: &str) -> bool {
        self.path.get_ident().map(|ident| ident == name).unwrap_or_default()
    }
}

fn expand_data_item(input: TokenStream) -> TokenStream {
    let mut item = parse_struct(input);

    let crate_name = crate_name();
    let vis = &item.vis;
    let Lifetimes {
        item_lifetime,
        ext_lifetime,
        ext_generics,
    } = get_lifetimes(&item);
    let (impl_generics, type_generics, where_clause) = item.generics.split_for_impl();
    let item_type_name = &item.ident;

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

    let system_data_attr = extract_attr(&mut item.attrs, "system_data");
    let system_data_defs = system_data_attr.map(|attr| {
        let type_name = attr.parse_args::<Type>().expect("Cannot parse system data name");
        let lifetime = syn::Lifetime::new("'a", Span::call_site());
        let storages = storages(&lifetime, &item);
        let view_store_lifetime = syn::Lifetime::new("'b", Span::call_site());
        let MainViews {
            view_type,
            view_ret,
            view_mut_type,
            view_mut_ret
        } = storages_main_views(&view_store_lifetime, &lifetime, &item);

        quote! {
            #vis type #type_name<#lifetime> = #storages;

            impl<#lifetime, #view_store_lifetime: #lifetime> #crate_name::MainView<#lifetime> for #type_name<#view_store_lifetime> {
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
    let storages_ref = storages_refs(&ext_lifetime, &item_lifetime, &item);

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

fn parse_struct(input: TokenStream) -> syn::ItemStruct {
    let item = syn::parse2(input).expect("Failed parse input token stream");
    match item {
        Item::Struct(item) => item,
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

fn get_attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a syn::Attribute> {
    attrs.iter().find(|attr| attr.is_name(name))
}

fn extract_attr(attrs: &mut Vec<Attribute>, name: &str) -> Option<syn::Attribute> {
    attrs
        .iter()
        .enumerate()
        .find_map(|(i, attr)| if attr.is_name(name) { Some(i) } else { None })
        .map(|idx| attrs.remove(idx))
}

fn crate_name() -> Ident {
    Ident::new(CRATE_NAME, Span::call_site())
}

fn storages(store_lifetime: &syn::Lifetime, item: &syn::ItemStruct) -> TokenStream {
    let storages: Vec<_> = item
        .fields
        .iter()
        .map(|field| {
            let (is_mut, field_type) = match &field.ty {
                syn::Type::Reference(ref_type) => (ref_type.mutability.is_some(), (*ref_type.elem).clone()),
                _ => (false, field.ty.clone()),
            };
            let is_resource = get_attr(&field.attrs, "resource").is_some();
            let crate_name = crate_name();
            let store = match (is_mut, is_resource) {
                (true, true) => quote! { #crate_name::specs::Write },
                (false, true) => quote! { #crate_name::specs::Read },
                (true, false) => quote! { #crate_name::specs::WriteStorage },
                (false, false) => quote! { #crate_name::specs::ReadStorage },
            };
            quote! { #store<#store_lifetime, #field_type> }
        })
        .collect();

    if storages.len() == 1 {
        storages.into_iter().next().unwrap()
    } else {
        quote! { (#(#storages),*) }
    }
}

fn storages_refs(store_lifetime: &syn::Lifetime, refs_lifetime: &syn::Lifetime, item: &syn::ItemStruct) -> TokenStream {
    let storages: Vec<_> = item
        .fields
        .iter()
        .map(|field| {
            let (is_mut, field_type) = match &field.ty {
                syn::Type::Reference(ref_type) => (ref_type.mutability.is_some(), (*ref_type.elem).clone()),
                _ => (false, field.ty.clone()),
            };
            let is_resource = get_attr(&field.attrs, "resource").is_some();
            let crate_name = crate_name();
            let store = match (is_mut, is_resource) {
                (true, true) => quote! { &#refs_lifetime mut #crate_name::specs::Write },
                (false, true) => quote! { &#refs_lifetime #crate_name::specs::Read },
                (true, false) => quote! { &#refs_lifetime mut #crate_name::specs::WriteStorage },
                (false, false) => quote! { &#refs_lifetime #crate_name::specs::ReadStorage },
            };
            quote! { #store<#store_lifetime, #field_type> }
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
    item: &syn::ItemStruct,
) -> MainViews {
    let mut view_indexes = vec![];
    let view_storages: Vec<_> = item
        .fields
        .iter()
        .enumerate()
        .filter_map(|(idx, field)| {
            let field_type = match &field.ty {
                syn::Type::Reference(ref_type) => {
                    if ref_type.mutability.is_none() {
                        Some((*ref_type.elem).clone())
                    } else {
                        None
                    }
                }
                _ => Some(field.ty.clone()),
            };

            field_type.map(|field_type| {
                let is_resource = get_attr(&field.attrs, "resource").is_some();
                let crate_name = crate_name();
                let store = if is_resource {
                    quote! { &#refs_lifetime #crate_name::specs::Read }
                } else {
                    quote! { &#refs_lifetime #crate_name::specs::ReadStorage }
                };
                view_indexes.push(idx);
                quote! { #store<#store_lifetime, #field_type> }
            })
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

    let (view_mut_type, view_mut_ret) = if view_indexes.len() == item.fields.len() {
        (quote! { () }, quote! { () })
    } else {
        let ret_tuple_fields: Vec<_> = item
            .fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let i = Literal::usize_unsuffixed(i);
                match &field.ty {
                    syn::Type::Reference(ref_type) if ref_type.mutability.is_some() => quote! { &mut self.#i },
                    _ => quote! { &self.#i },
                }
            })
            .collect();

        let ret = if ret_tuple_fields.len() == 1 {
            ret_tuple_fields.into_iter().next().unwrap()
        } else {
            quote! { (#(#ret_tuple_fields),*) }
        };

        (storages_refs(store_lifetime, refs_lifetime, item), ret)
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
    fn expand_macro() {
        let item = quote! {
            #[system_data(PhysicsSystemData)]
            #[derive(Clone, Copy)]
            struct PosVel<'a> {
                pos: &'a mut Pos,
                vel: &'a Vel,
            }
        };
        let output = expand_data_item(item).to_string();
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
type PhysicsSystemData < 'a > = ( specs_dsl :: specs :: WriteStorage < 'a , Pos > , specs_dsl :: specs :: ReadStorage < 'a , Vel > ) ; \
impl < 'a , 'b : 'a > specs_dsl :: MainView < 'a > for PhysicsSystemData < 'b > { \
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
}
