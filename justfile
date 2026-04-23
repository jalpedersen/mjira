install:
	cargo build --release
	cp target/release/jira ~/bin/mjira
	cp target/release/tempo ~/bin/mtempo
	cp target/release/snow ~/bin/msnow
