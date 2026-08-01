from pathlib import Path

path = Path("tools/issue233_apply.py")
source = path.read_text()
old = '''        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("market.sqlite");
        let path = path.to_str().unwrap();
        let order_id = {
            let mut market = MarketDb::open(path).unwrap();
'''
new = '''        let path = std::env::temp_dir().join(format!(
            "dawn-market-reopen-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path_string = path.to_string_lossy().into_owned();
        let order_id = {
            let mut market = MarketDb::open(&path_string).unwrap();
'''
if source.count(old) != 1:
    raise RuntimeError("expected tempfile reopen fixture exactly once")
source = source.replace(old, new)
old = "        let mut reopened = MarketDb::open(path).unwrap();\n"
new = "        let mut reopened = MarketDb::open(&path_string).unwrap();\n"
if source.count(old) != 1:
    raise RuntimeError("expected reopen call exactly once")
source = source.replace(old, new)
old = '''        assert_eq!(
            cancelled.return_item_command,
            Some(ReturnItemCommand {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: scrap(),
                quantity: 5,
            })
        );
    }
'''
new = '''        assert_eq!(
            cancelled.return_item_command,
            Some(ReturnItemCommand {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: scrap(),
                quantity: 5,
            })
        );
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
'''
if source.count(old) != 1:
    raise RuntimeError("expected reopen fixture tail exactly once")
path.write_text(source.replace(old, new))
