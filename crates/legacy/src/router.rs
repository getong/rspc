use std::{
    borrow::Borrow,
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    marker::PhantomData,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use futures::Stream;
use rspc_procedure::Procedures;
use serde_json::Value;
use specta::{datatype::FunctionResultVariant, DataType, TypeCollection};
use specta_typescript::{self as ts, datatype, export_named_datatype, Typescript};

use crate::{
    internal::{Procedure, ProcedureKind, ProcedureStore, RequestContext, ValueOrStream},
    Config, ExecError, ExportError,
};

#[cfg_attr(
    feature = "deprecated",
    deprecated = "This is replaced by `rspc::Router`. Refer to the `rspc::legacy` module for bridging a legacy router into a modern one."
)]
/// TODO
pub struct Router<TCtx = (), TMeta = ()>
where
    TCtx: 'static,
{
    pub(crate) config: Config,
    pub(crate) queries: ProcedureStore<TCtx>,
    pub(crate) mutations: ProcedureStore<TCtx>,
    pub(crate) subscriptions: ProcedureStore<TCtx>,
    pub(crate) type_map: TypeCollection,
    pub(crate) phantom: PhantomData<TMeta>,
}

// TODO: Move this out of this file
// TODO: Rename??
pub enum ExecKind {
    Query,
    Mutation,
}

impl<TCtx, TMeta> Router<TCtx, TMeta>
where
    TCtx: 'static,
{
    pub async fn exec(
        &self,
        ctx: TCtx,
        kind: ExecKind,
        key: String,
        input: Option<Value>,
    ) -> Result<Value, ExecError> {
        let (operations, kind) = match kind {
            ExecKind::Query => (&self.queries.store, ProcedureKind::Query),
            ExecKind::Mutation => (&self.mutations.store, ProcedureKind::Mutation),
        };

        match operations
            .get(&key)
            .ok_or_else(|| ExecError::OperationNotFound(key.clone()))?
            .exec
            .call(
                ctx,
                input.unwrap_or(Value::Null),
                RequestContext {
                    kind,
                    path: key.clone(),
                },
            )?
            .into_value_or_stream()
            .await?
        {
            ValueOrStream::Value(v) => Ok(v),
            ValueOrStream::Stream(_) => Err(ExecError::UnsupportedMethod(key)),
        }
    }

    pub async fn exec_subscription(
        &self,
        ctx: TCtx,
        key: String,
        input: Option<Value>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Value, ExecError>> + Send>>, ExecError> {
        match self
            .subscriptions
            .store
            .get(&key)
            .ok_or_else(|| ExecError::OperationNotFound(key.clone()))?
            .exec
            .call(
                ctx,
                input.unwrap_or(Value::Null),
                RequestContext {
                    kind: ProcedureKind::Subscription,
                    path: key.clone(),
                },
            )?
            .into_value_or_stream()
            .await?
        {
            ValueOrStream::Value(_) => Err(ExecError::UnsupportedMethod(key)),
            ValueOrStream::Stream(s) => Ok(s),
        }
    }

    pub fn arced(self) -> Arc<Self> {
        Arc::new(self)
    }

    #[deprecated = "Use `Self::type_map`"]
    pub fn typ_store(&self) -> TypeCollection {
        self.type_map.clone()
    }

    pub fn type_map(&self) -> TypeCollection {
        self.type_map.clone()
    }

    pub fn queries(&self) -> &BTreeMap<String, Procedure<TCtx>> {
        &self.queries.store
    }

    pub fn mutations(&self) -> &BTreeMap<String, Procedure<TCtx>> {
        &self.mutations.store
    }

    pub fn subscriptions(&self) -> &BTreeMap<String, Procedure<TCtx>> {
        &self.subscriptions.store
    }

    #[doc(hidden)] // Used for `rspc::legacy` interop
    pub fn into_parts(
        self,
    ) -> (
        BTreeMap<String, Procedure<TCtx>>,
        BTreeMap<String, Procedure<TCtx>>,
        BTreeMap<String, Procedure<TCtx>>,
        TypeCollection,
    ) {
        if self.config.export_bindings_on_build.is_some() || self.config.bindings_header.is_some() {
            panic!("Note: `rspc_legacy::Config` is ignored by `rspc::Router`. You should set the configuration on `rspc::Typescript` instead.");
        }

        (
            self.queries.store,
            self.mutations.store,
            self.subscriptions.store,
            self.type_map,
        )
    }

    #[allow(clippy::unwrap_used)] // TODO
    pub fn export_ts<TPath: AsRef<Path>>(&self, export_path: TPath) -> Result<(), ExportError> {
        let export_path = PathBuf::from(export_path.as_ref());
        if let Some(export_dir) = export_path.parent() {
            fs::create_dir_all(export_dir)?;
        }
        let mut file = File::create(export_path)?;
        if let Some(header) = &self.config.bindings_header {
            writeln!(file, "{}", header)?;
        }
        writeln!(file, "// This file was generated by [rspc](https://github.com/specta-rs/rspc). Do not edit this file manually.")?;

        let config = Typescript::new().bigint(
            ts::BigIntExportBehavior::FailWithReason(
                "rspc does not support exporting bigint types (i64, u64, i128, u128) because they are lossily decoded by `JSON.parse` on the frontend. Tracking issue: https://github.com/specta-rs/rspc/issues/93",
            )
        );

        let queries_ts = generate_procedures_ts(&config, &self.queries.store, &self.type_map);
        let mutations_ts = generate_procedures_ts(&config, &self.mutations.store, &self.type_map);
        let subscriptions_ts =
            generate_procedures_ts(&config, &self.subscriptions.store, &self.type_map);

        // TODO: Specta API
        writeln!(
            file,
            r#"
export type Procedures = {{
    queries: {queries_ts},
    mutations: {mutations_ts},
    subscriptions: {subscriptions_ts}
}};"#
        )?;

        // Generate type exports (non-Procedures)
        for export in self
            .type_map
            .into_iter()
            .map(|(_, ty)| export_named_datatype(&config, ty, &self.type_map).unwrap())
        {
            writeln!(file, "\n{}", export)?;
        }

        Ok(())
    }
}

// TODO: Move this out into a Specta API
fn generate_procedures_ts<Ctx>(
    config: &Typescript,
    procedures: &BTreeMap<String, Procedure<Ctx>>,
    type_map: &TypeCollection,
) -> String {
    match procedures.len() {
        0 => "never".to_string(),
        _ => procedures
            .iter()
            .map(|(key, operation)| {
                let input = match &operation.ty.arg_ty {
                    DataType::Tuple(def)
                        // This condition is met with an empty enum or `()`.
                        if def.elements().is_empty() =>
                    {
                        "never".into()
                    }
                    #[allow(clippy::unwrap_used)] // TODO
                    ty => datatype(config,  &FunctionResultVariant::Value(ty.clone()), type_map).unwrap(),
                };
                #[allow(clippy::unwrap_used)] // TODO
                let result_ts = datatype(
                    config,
                    &FunctionResultVariant::Value(operation.ty.result_ty.clone()),
                    type_map,
                )
                .unwrap();

                // TODO: Specta API
                format!(
                    r#"
        {{ key: "{key}", input: {input}, result: {result_ts} }}"#
                )
            })
            .collect::<Vec<_>>()
            .join(" | "),
    }
}
