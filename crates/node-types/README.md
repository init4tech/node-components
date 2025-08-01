# Signet Node Types

This crate provides parameterizations and conveneniences for the Signet node's
use of reth's internal generics. E.g. [`NodePrimitives`] and [`NodeTypes`].

It also provides a [`NodeTypesDbTrait`] to aggregate several trait constraints
on the database type. This is then used in the node and in `signet-db`.

This crate is mostly shims. It is not intended to be used outside of the
Signet node and `signet-db` crates.
