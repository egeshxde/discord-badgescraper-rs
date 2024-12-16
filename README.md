# Discord Badge Scraper

Hey there! 👋 This tool lets you scrape Discord user badges from a server in real-time. It's built in Rust and uses WebSockets to interact with Discord's Gateway. If you're curious about what badges users have (like Partner, HypeSquad, Bug Hunter, etc.), this script will make it easy for you to grab that info.

## Features

- Fetches badges like Partner, Early Supporter, Bug Hunter, etc.
- Writes the data to a `results.txt` file.
- Handles real-time WebSocket communication with Discord.

## Usage

1. Clone this repo.
2. Install Rust if you don’t have it already.
3. Create a `config.json` file in the root directory and add this:
   
```json
   {
       "token": "YOUR_DISCORD_ACC_TOKEN",
       "guild_id": "TARGET_GUILD_ID",
       "channel_id": "A_TEXT_CHANNEL_ID_FROM_THE_GUILD"
   }
```


Run the scraper:
```bash
cargo run
```

Check the `results.txt` file for the scraped user data.
