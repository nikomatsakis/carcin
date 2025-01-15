use std::path::PathBuf;

use gluegun_core::{
    codegen::{CodeWriter, DirBuilder},
    idl::{
        Enum, Field, FunctionInput, Idl, Item, Method, MethodCategory, Name, QualifiedName, Record,
        Resource, Scalar, SelfKind, Signature, Ty, TypeKind, Variant,
    },
};

use crate::util;

pub(crate) struct RustCodeGenerator<'idl> {
    idl: &'idl Idl,
}

impl<'idl> RustCodeGenerator<'idl> {
    pub(crate) fn new(idl: &'idl Idl) -> Self {
        Self { idl }
    }

    pub(crate) fn generate(mut self, mut dir: DirBuilder<'_>) -> anyhow::Result<()> {
        let mut lib_rs = dir.add_file("src/lib.rs")?;
        for (qname, item) in self.idl.definitions() {
            self.generate_item(&mut lib_rs, qname, item)?;
        }
        Ok(())
    }

    fn generate_item(
        &mut self,
        lib_rs: &mut CodeWriter<'_>,
        qname: &QualifiedName,
        item: &Item,
    ) -> anyhow::Result<()> {
        match item {
            Item::Resource(resource) => self.generate_resource(lib_rs, qname, resource),
            Item::Record(record) => self.generate_record(lib_rs, qname, record),
            Item::Variant(variant) => self.generate_variant(lib_rs, qname, variant),
            Item::Enum(an_enum) => self.generate_enum(lib_rs, qname, an_enum),
            Item::Function(_) => {
                // Skip functions for now. We will collect them and generate them as static methods.
                Ok(())
            }
            _ => anyhow::bail!("unsupported item: {item:?}"),
        }
    }

    fn generate_resource(
        &self,
        lib_rs: &mut CodeWriter<'_>,
        qname: &QualifiedName,
        resource: &Resource,
    ) -> Result<(), anyhow::Error> {
        for method in resource.methods() {
            self.generate_method(lib_rs, qname, method)?;
        }
        Ok(())
    }

    fn generate_record(
        &self,
        lib_rs: &mut CodeWriter<'_>,
        qname: &QualifiedName,
        record: &Record,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    fn generate_variant(
        &self,
        lib_rs: &mut CodeWriter<'_>,
        qname: &QualifiedName,
        variant: &Variant,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    fn generate_enum(
        &self,
        lib_rs: &mut CodeWriter<'_>,
        qname: &QualifiedName,
        an_enum: &Enum,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }

    fn generate_method(
        &self,
        lib_rs: &mut CodeWriter<'_>,
        qname: &QualifiedName,
        method: &Method,
    ) -> anyhow::Result<()> {
        self.generate_native_function(
            lib_rs,
            qname,
            qname,
            method.name(),
            method.category(),
            method.signature(),
        )
    }

    /// Generate a native function definition that will be the backing function for a Java method.
    ///
    /// # Parameters
    ///
    /// * `lib_rs`, write-stream for the `lib.rs` file
    /// * `rust_qname`, qname of the `Resource` type or, for free functions, the containing module
    /// * `java_qname`, the qname of the Java class containing the method; often the same as `rust_qname` but (e.g. for free functions) not always
    /// * `fn_name`, the name of the method/function
    /// * `method_category`, the category of method (e.g., static etc). Static for free functions.
    /// * `signature`, types of inputs/outputs apart from `self`
    fn generate_native_function(
        &self,
        lib_rs: &mut CodeWriter<'_>,
        rust_qname: &QualifiedName,
        java_qname: &QualifiedName,
        fn_name: &Name,
        method_category: &MethodCategory,
        signature: &Signature,
    ) -> anyhow::Result<()> {
        write!(lib_rs, "const _: () = {{")?;
        write!(
            lib_rs,
            "#[duchess::java_function({class_dot_name}::{fn_name})]",
            class_dot_name = util::class_dot_name(java_qname)
        )?;
        write!(lib_rs, "fn {fn_name}(")?;

        match method_category {
            MethodCategory::Constructor => {}
            MethodCategory::BuilderMethod(_self_kind)
            | MethodCategory::InstanceMethod(_self_kind) => {
                write!(lib_rs, "_self: &duchess::JavaObject")?; // FIXME
            }
            MethodCategory::StaticMethod => {}
            _ => anyhow::bail!("unsupported method category: {method_category:?}"),
        }

        for input in signature.inputs() {
            let name = input.name();
            let ty = input.ty();
            write!(lib_rs, "{name}: {ty},", ty = self.rust_parameter_ty(ty))?;
        }

        write!(lib_rs, ") -> duchess::Result<> {{")?;

        // Fn body is just a call to the underlying Rust function
        write!(lib_rs, "{m}::{fn_name}(", m = rust_qname.colon_colon())?;
        write!(lib_rs, "{m}::{fn_name}(", m = rust_qname.colon_colon())?;
        write!(lib_rs, ")")?;

        write!(lib_rs, "}}")?;
        write!(lib_rs, "}}")?;
        Ok(())
    }

    fn rust_parameter_ty(&self, ty: &Ty) -> String {
        match ty.kind() {
            TypeKind::Map { key, value } => todo!(),
            TypeKind::Vec { element } => todo!(),
            TypeKind::Set { element } => todo!(),
            TypeKind::Path => todo!(),
            TypeKind::String => todo!(),
            TypeKind::Option { element } => todo!(),
            TypeKind::Result { ok, err } => todo!(),
            TypeKind::Tuple { elements } => todo!(),
            TypeKind::Scalar(scalar) => scalar.to_string(),
            TypeKind::Future { output } => todo!(),
            TypeKind::Error => todo!(),
            TypeKind::UserType { qname } => todo!(),
            _ => todo!(),
        }
    }
}
