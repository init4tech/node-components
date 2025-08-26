#[path = "./common/mod.rs"]
mod test_common;

use signet_db::RuWriter;

#[test]
fn test_ru_writer() {
    let factory = test_common::create_test_provider_factory();

    let writer = factory.provider_rw().unwrap();

    dbg!(writer.last_block_number().unwrap());
}
