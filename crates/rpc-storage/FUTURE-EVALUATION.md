# Future Evaluation Notes

## Vec Collection at API Boundary

Several endpoints (`eth_getBlockReceipts`, `debug_traceBlockByNumber`, etc.)
collect results into a `Vec` before returning. This is required because
`ajj::ResponsePayload` expects an owned `Serialize` value — there is no way
to feed items to the serializer via an iterator or streaming interface.

If `ajj` adds support for streaming serialization (e.g. accepting an
`Iterator<Item: Serialize>` or a `Stream`), these endpoints could be
refactored to avoid the intermediate allocation. Until then, the `Vec`
collection is the necessary approach at the API boundary.
