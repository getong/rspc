use std::{
    any::{Any, TypeId},
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    ConcreteArg, ExecError, Key, KeyDefinition, Operation, SubscriptionOperation, TSDependency,
};

/// TODO
pub struct Router<
    TCtx = (),
    TMeta = (),
    TQueryKey = &'static str,
    TMutationKey = &'static str,
    TSubscriptionKey = &'static str,
> where
    TCtx: Send + Sync + 'static,
    TMeta: Send + Sync + 'static,
    TQueryKey: KeyDefinition,
    TMutationKey: KeyDefinition,
    TSubscriptionKey: KeyDefinition,
{
    pub(crate) query: Operation<TQueryKey, TCtx>,
    pub(crate) mutation: Operation<TMutationKey, TCtx>,
    pub(crate) subscription: SubscriptionOperation<TSubscriptionKey, ()>,
    pub(crate) phantom: PhantomData<TMeta>,
}

impl<TCtx, TMeta, TQueryKey, TMutationKey, TSubscriptionKey>
    Router<TCtx, TMeta, TQueryKey, TMutationKey, TSubscriptionKey>
where
    TCtx: Send + Sync + 'static,
    TMeta: Send + Sync + 'static,
    TQueryKey: KeyDefinition,
    TMutationKey: KeyDefinition,
    TSubscriptionKey: KeyDefinition,
{
    pub async fn exec_query<TArg, TKey>(
        &self,
        ctx: TCtx,
        key: TKey,
        arg: TArg,
    ) -> Result<Value, ExecError>
    where
        TArg: Send + Sync + 'static,
        TKey: Key<TQueryKey, TArg>,
    {
        let definition = self
            .query
            .get(key.to_val())
            .ok_or(ExecError::OperationNotFound)?;
        let arg = match TypeId::of::<TArg>() == TypeId::of::<Value>() {
            // SAFETY: We check the TypeId's match before `transmute_copy`. We use this method as I couldn't implement a trait which wouldn't overlap to abstract this into.
            true => {
                // We are using runtime specialization because I could not come up with a trait which wouldn't overlap to abstract this into.
                let v = (&mut Some(arg) as &mut dyn Any)
                    .downcast_mut::<Option<Value>>()
                    .unwrap()
                    .take()
                    .unwrap();
                ConcreteArg::Value(v)
            }
            false => ConcreteArg::Unknown(Box::new(arg)),
        };

        definition(ctx, arg)?.await
    }

    #[allow(dead_code)]
    pub(crate) async fn exec_query_unsafe(
        &self,
        ctx: TCtx,
        key: String,
        arg: Value,
    ) -> Result<Value, ExecError> {
        let definition = self
            .query
            .get(TQueryKey::from_str(key)?)
            .ok_or(ExecError::OperationNotFound)?;
        definition(ctx, ConcreteArg::Value(arg))?.await
    }

    pub async fn exec_mutation<TArg, TKey>(
        &self,
        ctx: TCtx,
        key: TKey,
        arg: TArg,
    ) -> Result<Value, ExecError>
    where
        TArg: Send + Sync + 'static,
        TKey: Key<TMutationKey, TArg>,
    {
        let definition = self
            .mutation
            .get(key.to_val())
            .ok_or(ExecError::OperationNotFound)?;
        let arg = match TypeId::of::<TArg>() == TypeId::of::<Value>() {
            true => {
                // We are using runtime specialization because I could not come up with a trait which wouldn't overlap to abstract this into.
                let v = (&mut Some(arg) as &mut dyn Any)
                    .downcast_mut::<Option<Value>>()
                    .unwrap()
                    .take()
                    .unwrap();
                ConcreteArg::Value(v)
            }
            false => ConcreteArg::Unknown(Box::new(arg)),
        };

        definition(ctx, arg)?.await
    }

    #[allow(dead_code)]
    pub(crate) async fn exec_mutation_unsafe(
        &self,
        ctx: TCtx,
        key: String,
        arg: Value,
    ) -> Result<Value, ExecError> {
        let definition = self
            .mutation
            .get(TMutationKey::from_str(key)?)
            .ok_or(ExecError::OperationNotFound)?;
        definition(ctx, ConcreteArg::Value(arg))?.await
    }

    #[allow(dead_code)]
    pub(crate) async fn exec_subscription_unsafe(&self, key: String) -> Result<(), ExecError> {
        let definition = self
            .subscription
            .get(TSubscriptionKey::from_str(key)?)
            .ok_or(ExecError::OperationNotFound)?;
        Ok(definition(()))
    }

    // TODO: Don't use `Box<Error>` as return type.
    pub fn export<TPath: AsRef<Path>>(
        &self,
        export_path: TPath,
        // TODO: New error type
    ) -> Result<(), Box<dyn std::error::Error>> {
        let export_path = PathBuf::from(export_path.as_ref());
        fs::create_dir_all(&export_path)?;
        let mut file = File::create(export_path.clone().join("index.ts"))?;
        writeln!(file, "// This file was generated by [rspc](https://github.com/oscartbeaumont/rspc). Do not edit this file manually.")?;

        let mut dependencies = BTreeSet::<TSDependency>::new();

        let mut query_buf = Vec::new();
        self.query
            .export(&mut dependencies, &mut query_buf, export_path.clone())?;

        let mut mutation_buf = Vec::new();
        self.mutation
            .export(&mut dependencies, &mut mutation_buf, export_path)?;

        for dep in dependencies.into_iter() {
            writeln!(
                file,
                "import type {{ {} }} from {:?};",
                dep.ts_name.clone(),
                format!("./{}", dep.ts_name)
            )?;
        }

        writeln!(
            file,
            "\nexport interface Operations {{ queries: Queries, mutations: Mutations, subscriptions: Subscriptions }}"
        )?;

        write!(file, "\nexport type Queries =")?;
        file.write_all(&query_buf)?;
        writeln!(file, ";")?;

        write!(file, "\nexport type Mutations =")?;
        file.write_all(&mutation_buf)?;
        writeln!(file, ";")?;

        // TODO
        write!(file, "\nexport type Subscriptions = never;")?;

        Ok(())
    }
}
