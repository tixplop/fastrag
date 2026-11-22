use crate::common::dossier_test;
use fastrag::config::Config;

#[test]
fn socle_charge_une_configuration_du_disque() {
    let d = dossier_test("socle");
    let f = d.join("c.toml");
    std::fs::write(&f, "port = 1234\n").unwrap();
    assert_eq!(Config::charger(Some(&f)).unwrap().port, 1234);
}
