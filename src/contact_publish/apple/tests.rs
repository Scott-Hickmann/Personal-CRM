use super::*;

#[test]
fn recognizes_icloud_carddav_container() {
    let container = AppleContainer {
        id: "icloud-account".into(),
        name: "iCloud".into(),
        kind: "com.apple.account.CardDAV".into(),
    };

    assert!(is_icloud(&container));
}

#[test]
fn rejects_non_icloud_contact_containers() {
    let local = AppleContainer {
        id: "local".into(),
        name: "On My Mac".into(),
        kind: "local".into(),
    };
    let other_carddav = AppleContainer {
        id: "other-account".into(),
        name: "Fastmail".into(),
        kind: "com.apple.account.CardDAV".into(),
    };

    assert!(!is_icloud(&local));
    assert!(!is_icloud(&other_carddav));
}

#[test]
fn recognizes_icloud_account_type() {
    let container = AppleContainer {
        id: "icloud-account".into(),
        name: "Contacts".into(),
        kind: "com.apple.account.iCloud".into(),
    };

    assert!(is_icloud(&container));
}

#[test]
fn exports_company_type_and_labeled_fields_from_read_only_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("AddressBook-v22.abcddb");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE ZABCDRECORD (
                Z_PK INTEGER PRIMARY KEY, ZUNIQUEID TEXT, ZDISPLAYFLAGS INTEGER,
                ZTITLE TEXT, ZFIRSTNAME TEXT, ZMIDDLENAME TEXT, ZLASTNAME TEXT,
                ZSUFFIX TEXT, ZNICKNAME TEXT, ZORGANIZATION TEXT,
                ZDEPARTMENT TEXT, ZJOBTITLE TEXT
             );
             CREATE TABLE ZABCDEMAILADDRESS (
                Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZADDRESS TEXT,
                ZLABEL TEXT, ZORDERINGINDEX INTEGER
             );
             CREATE TABLE ZABCDPHONENUMBER (
                Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZFULLNUMBER TEXT,
                ZLABEL TEXT, ZORDERINGINDEX INTEGER
             );
             INSERT INTO ZABCDRECORD VALUES
                (1, 'apple-1', 0, '', 'Alex', '', 'Example', '', '',
                 'Example Inc', 'R&D', 'Engineer'),
                (2, 'apple-2', 1, '', '', '', '', '', '', 'Example Inc', '', '');
             INSERT INTO ZABCDEMAILADDRESS VALUES
                (1, 1, 'alex@example.com', '$!<Work>!$', 0);
             INSERT INTO ZABCDPHONENUMBER VALUES
                (1, 1, '555-0100', '$!<Mobile>!$', 0);",
        )
        .unwrap();
    drop(connection);

    let exported = contacts(&path, "local").unwrap();
    assert_eq!(exported.len(), 2);
    assert_eq!(exported[0].id, "apple-1");
    assert!(!exported[0].is_company);
    assert_eq!(exported[0].emails[0].label.as_deref(), Some("$!<Work>!$"));
    assert_eq!(exported[0].phones[0].value, "555-0100");
    assert!(exported[1].is_company);
}

#[test]
fn contact_change_token_tracks_core_data_versions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("AddressBook-v22.abcddb");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE ZABCDRECORD (Z_PK INTEGER PRIMARY KEY, Z_OPT INTEGER);
             CREATE TABLE ZABCDEMAILADDRESS (
                Z_PK INTEGER PRIMARY KEY, Z_OPT INTEGER, ZOWNER INTEGER
             );
             CREATE TABLE ZABCDPHONENUMBER (
                Z_PK INTEGER PRIMARY KEY, Z_OPT INTEGER, ZOWNER INTEGER
             );
             INSERT INTO ZABCDRECORD VALUES (1, 1);
             INSERT INTO ZABCDEMAILADDRESS VALUES (1, 1, 1);",
        )
        .unwrap();

    let before = change_token(&path, "local").unwrap().unwrap();
    connection
        .execute("UPDATE ZABCDEMAILADDRESS SET Z_OPT=2 WHERE Z_PK=1", [])
        .unwrap();
    let after = change_token(&path, "local").unwrap().unwrap();

    assert_ne!(before, after);
}
