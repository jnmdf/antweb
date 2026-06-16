use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Meta, parse_macro_input};

#[proc_macro_derive(SeaModel, attributes(rename))]
pub fn derive_sea_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => panic!("SeaModel only supports Named Structs"),
        },
        _ => panic!("SeaModel only supports Structs"),
    };

    let mut column_names = Vec::new();
    let mut field_idents = Vec::new();

    // 1. 编译期解析字段名与强大的 #[rename = "..."] 属性
    for field in fields {
        let ident = field.ident.unwrap();
        let mut name_str = ident.to_string();

        for attr in field.attrs {
            if attr.path().is_ident("rename")
                && let Meta::NameValue(meta) = attr.meta
                && let syn::Expr::Lit(expr_lit) = meta.value
                && let syn::Lit::Str(lit_str) = expr_lit.lit
            {
                name_str = lit_str.value();
            }
        }
        column_names.push(name_str);
        field_idents.push(ident);
    }

    // 2. 同时生成 INSERT（纯数组）和 UPDATE（键值对二元组）两条高性能 Move 路径
    let expanded = quote! {
        impl #struct_name {
            /// 供 INSERT 使用：提取精准的列名别名数组
            pub fn columns_auto() -> Vec<sea_query::Alias> {
                vec![ #( sea_query::Alias::new(#column_names) ),* ]
            }

            /// 供高效大批量 INSERT 使用：全线 Move 语义转移所有权，0次内存克隆
            pub fn into_row_values(self) -> Vec<sea_query::Expr> {
                vec![ #( sea_query::Expr::val(self.#field_idents) ),* ]
            }

            /// 供 UPDATE 和动态过滤使用：精准返回拥有所有权的 (列名, 强类型值) 二元组
            /// 全线保持最高级别的所有权解构 Move 语义
            pub fn into_column_values(self) -> Vec<(sea_query::Alias, sea_query::Expr)> {
                vec![ #( (sea_query::Alias::new(#column_names), sea_query::Expr::val(self.#field_idents)) ),* ]
            }
        }
    };

    TokenStream::from(expanded)
}
