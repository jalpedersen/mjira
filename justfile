install:
	cargo build --release
	cp target/release/jira ~/bin/mjira
