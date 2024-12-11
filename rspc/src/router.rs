use std::{
    borrow::{Borrow, Cow},
    collections::{BTreeMap, HashMap},
    fmt,
    sync::Arc,
};

use specta::TypeCollection;

use rspc_core::Procedures;

use crate::{types::TypesOrType, Procedure2, ProcedureKind, State, Types};

/// TODO: Examples exporting types and with `rspc_axum`
pub struct Router2<TCtx = ()> {
    setup: Vec<Box<dyn FnOnce(&mut State) + 'static>>,
    types: TypeCollection,
    procedures: BTreeMap<Vec<Cow<'static, str>>, Procedure2<TCtx>>,
    errors: Vec<Error>,
}

impl<TCtx> Default for Router2<TCtx> {
    fn default() -> Self {
        Self {
            setup: Default::default(),
            types: Default::default(),
            procedures: Default::default(),
            errors: vec![],
        }
    }
}

impl<TCtx> Router2<TCtx> {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: Enforce unique across all methods (query, subscription, etc). Eg. `insert` should yield error if key already exists.
    #[cfg(feature = "unstable")]
    pub fn procedure(
        mut self,
        key: impl Into<Cow<'static, str>>,
        mut procedure: Procedure2<TCtx>,
    ) -> Self {
        let key = key.into();

        if self.procedures.keys().any(|k| k[0] == key) {
            self.errors.push(Error::DuplicateProcedures(vec![key]))
        } else {
            self.setup.extend(procedure.setup.drain(..));
            self.procedures.insert(vec![key], procedure);
        }

        self
    }

    // TODO: Document the order this is run in for `build`
    #[cfg(feature = "unstable")]
    pub fn setup(mut self, func: impl FnOnce(&mut State) + 'static) -> Self {
        self.setup.push(Box::new(func));
        self
    }

    // TODO: Yield error if key already exists
    pub fn nest(mut self, prefix: impl Into<Cow<'static, str>>, mut other: Self) -> Self {
        let prefix = prefix.into();

        dbg!(&self.procedures.keys().collect::<Vec<_>>());
        if self.procedures.keys().any(|k| k[0] == prefix) {
            self.errors.push(Error::DuplicateProcedures(vec![prefix]));
        } else {
            self.setup.append(&mut other.setup);

            self.procedures
                .extend(other.procedures.into_iter().map(|(k, v)| {
                    let mut new_key = vec![prefix.clone()];
                    new_key.extend(k);
                    (new_key, v)
                }));

            self.errors
                .extend(other.errors.into_iter().map(|e| match e {
                    Error::DuplicateProcedures(key) => {
                        let mut new_key = vec![prefix.clone()];
                        new_key.extend(key);
                        Error::DuplicateProcedures(new_key)
                    }
                }));
        }

        self
    }

    // TODO: Yield error if key already exists
    pub fn merge(mut self, mut other: Self) -> Self {
        let error_count = self.errors.len();

        for other_proc in other.procedures.keys() {
            if self.procedures.get(other_proc).is_some() {
                self.errors
                    .push(Error::DuplicateProcedures(other_proc.clone()));
            }
        }

        if self.errors.len() > error_count {
            self.setup.append(&mut other.setup);
            self.procedures.extend(other.procedures.into_iter());
            self.errors.extend(other.errors);
        }

        self
    }

    pub fn build(
        self,
    ) -> Result<
        (
            impl Borrow<Procedures<TCtx>> + Into<Procedures<TCtx>> + fmt::Debug,
            Types,
        ),
        Vec<Error>,
    > {
        self.build_with_state_inner(State::default())
    }

    #[cfg(feature = "unstable")]
    pub fn build_with_state(
        self,
        state: State,
    ) -> Result<
        (
            impl Borrow<Procedures<TCtx>> + Into<Procedures<TCtx>> + fmt::Debug,
            Types,
        ),
        Vec<Error>,
    > {
        self.build_with_state_inner(state)
    }

    fn build_with_state_inner(
        self,
        mut state: State,
    ) -> Result<
        (
            impl Borrow<Procedures<TCtx>> + Into<Procedures<TCtx>> + fmt::Debug,
            Types,
        ),
        Vec<Error>,
    > {
        if self.errors.len() > 0 {
            return Err(self.errors);
        }

        for setup in self.setup {
            setup(&mut state);
        }
        let state = Arc::new(state);

        let mut procedure_types = BTreeMap::new();
        let procedures = self
            .procedures
            .into_iter()
            .map(|(key, p)| {
                let mut current = &mut procedure_types;
                // TODO: if `key.len()` is `0` we might run into issues here. It shouldn't but probs worth protecting.
                for part in &key[..(key.len() - 1)] {
                    let a = current
                        .entry(part.clone())
                        .or_insert_with(|| TypesOrType::Types(Default::default()));
                    match a {
                        TypesOrType::Type(_) => unreachable!(), // TODO: Confirm this is unreachable
                        TypesOrType::Types(map) => current = map,
                    }
                }
                current.insert(key[key.len() - 1].clone(), TypesOrType::Type(p.ty));

                (get_flattened_name(&key), (p.inner)(state.clone()))
            })
            .collect::<HashMap<_, _>>();

        struct Impl<TCtx>(Procedures<TCtx>);
        impl<TCtx> Into<Procedures<TCtx>> for Impl<TCtx> {
            fn into(self) -> Procedures<TCtx> {
                self.0
            }
        }
        impl<TCtx> Borrow<Procedures<TCtx>> for Impl<TCtx> {
            fn borrow(&self) -> &Procedures<TCtx> {
                &self.0
            }
        }
        impl<TCtx> fmt::Debug for Impl<TCtx> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{:?}", self.0)
            }
        }

        Ok((
            Impl::<TCtx>(procedures),
            Types {
                types: self.types,
                procedures: procedure_types,
            },
        ))
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    DuplicateProcedures(Vec<Cow<'static, str>>),
}

impl<TCtx> fmt::Debug for Router2<TCtx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let procedure_keys = |kind: ProcedureKind| {
            self.procedures
                .iter()
                .filter(move |(_, p)| p.ty.kind == kind)
                .map(|(k, _)| k.join("."))
                .collect::<Vec<_>>()
        };

        f.debug_struct("Router")
            .field("queries", &procedure_keys(ProcedureKind::Query))
            .field("mutations", &procedure_keys(ProcedureKind::Mutation))
            .field(
                "subscriptions",
                &procedure_keys(ProcedureKind::Subscription),
            )
            .finish()
    }
}

impl<'a, TCtx> IntoIterator for &'a Router2<TCtx> {
    type Item = (&'a Vec<Cow<'static, str>>, &'a Procedure2<TCtx>);
    type IntoIter = std::collections::btree_map::Iter<'a, Vec<Cow<'static, str>>, Procedure2<TCtx>>;

    fn into_iter(self) -> Self::IntoIter {
        self.procedures.iter()
    }
}

#[cfg(not(feature = "nolegacy"))]
impl<TCtx> From<crate::legacy::Router<TCtx>> for Router2<TCtx> {
    fn from(router: crate::legacy::Router<TCtx>) -> Self {
        crate::interop::legacy_to_modern(router)
    }
}

#[cfg(not(feature = "nolegacy"))]
impl<TCtx> Router2<TCtx> {
    pub(crate) fn interop_procedures(
        &mut self,
    ) -> &mut BTreeMap<Vec<Cow<'static, str>>, Procedure2<TCtx>> {
        &mut self.procedures
    }

    pub(crate) fn interop_types(&mut self) -> &mut TypeCollection {
        &mut self.types
    }
}

fn get_flattened_name(name: &Vec<Cow<'static, str>>) -> Cow<'static, str> {
    if name.len() == 1 {
        // By cloning we are ensuring we passthrough to the `Cow` to avoid cloning if this is a `&'static str`.
        // Doing `.join` will always produce a new `String` removing the `&'static str` optimization.
        name[0].clone()
    } else {
        name.join(".").to_string().into()
    }
}

#[cfg(test)]
mod test {
    use rspc_core::ResolverError;
    use serde::Serialize;
    use specta::Type;

    use super::*;

    #[test]
    fn errors() {
        let router = <Router2>::new()
            .procedure(
                "abc",
                Procedure2::builder().query(|_, _: ()| async { Ok::<_, Infallible>(()) }),
            )
            .procedure(
                "abc",
                Procedure2::builder().query(|_, _: ()| async { Ok::<_, Infallible>(()) }),
            );

        assert_eq!(
            router.build().unwrap_err(),
            vec![Error::DuplicateProcedures(vec!["abc".into()])]
        );

        let router = <Router2>::new()
            .procedure(
                "abc",
                Procedure2::builder().query(|_, _: ()| async { Ok::<_, Infallible>(()) }),
            )
            .merge(<Router2>::new().procedure(
                "abc",
                Procedure2::builder().query(|_, _: ()| async { Ok::<_, Infallible>(()) }),
            ));

        assert_eq!(
            router.build().unwrap_err(),
            vec![Error::DuplicateProcedures(vec!["abc".into()])]
        );

        let router = <Router2>::new()
            .nest(
                "abc",
                <Router2>::new().procedure(
                    "kjl",
                    Procedure2::builder().query(|_, _: ()| async { Ok::<_, Infallible>(()) }),
                ),
            )
            .nest(
                "abc",
                <Router2>::new().procedure(
                    "def",
                    Procedure2::builder().query(|_, _: ()| async { Ok::<_, Infallible>(()) }),
                ),
            );

        assert_eq!(
            router.build().unwrap_err(),
            vec![Error::DuplicateProcedures(vec!["abc".into()])]
        );
    }

    #[derive(Type, Debug)]
    pub enum Infallible {}

    impl fmt::Display for Infallible {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{self:?}")
        }
    }

    impl Serialize for Infallible {
        fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            unreachable!()
        }
    }

    impl std::error::Error for Infallible {}

    impl crate::modern::Error for Infallible {
        fn into_resolver_error(self) -> ResolverError {
            unreachable!()
        }
    }
}
