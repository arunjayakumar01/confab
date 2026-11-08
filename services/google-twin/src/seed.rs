//! Deterministic seed: one workspace on `prism.test`, five humans, seven spaces,
//! about three hundred messages over the last seven days, threads, reactions,
//! read states and two image attachments. Same content every start; only the
//! clock moves.

use crate::config::Config;
use crate::state::{Blob, Store, CAL_PRIMARY, CAL_TEAM};
use crate::util::{fmt_time, DetRng};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use confab_proto::calendar::{Attendee, CalEvent, ConferenceData, EntryPoint, EventTime};
use confab_proto::chat::{
    Attachment, AttachmentDataRef, Emoji, Message, Reaction, Space, Thread, User,
};

pub const DOMAIN: &str = "prism.test";

/// (numeric id, display name, email local part)
pub const USERS: &[(&str, &str, &str)] = &[
    ("1001", "Arun Jayakumar", "arun"),
    ("1002", "Priya Nair", "priya"),
    ("1003", "Rahul Menon", "rahul"),
    ("1004", "Meera Iyer", "meera"),
    ("1005", "Dev Sharma", "dev"),
];

const AVATAR_COLORS: &[&str] = &["#5b6cff", "#d9480f", "#0b7285", "#862e9c", "#2b8a3e"];

/// A conversation: the first line is the message, the rest are thread replies.
/// Senders are 1-based indexes into `USERS`.
type Conv = &'static [(u8, &'static str)];

struct SpaceDef {
    id: &'static str,
    display_name: Option<&'static str>,
    space_type: &'static str,
    description: &'static str,
    members: &'static [u8],
    managers: &'static [u8],
    convs: &'static [Conv],
}

const GENERAL: &[Conv] = &[
    &[(1, "Morning all. Sprint review is Thursday 4pm, same room as last time.")],
    &[(4, "Reminder that the customer call with Kaveri Analytics moved to **Wednesday 11:30**. Invite updated.")],
    &[(2, "Coffee machine on the second floor is fixed. The descaling light was the whole problem.")],
    &[(5, "Office wifi is getting a new SSID tonight: `prism-5g`. Same password, see the wiki.")],
    &[(3, "Anyone else getting logged out of the staging dashboard every few minutes?"), (2, "Yes, since yesterday. I think the session cookie is being set on the wrong domain."), (5, "That is the new ingress. I changed the host header rewrite, will fix in an hour."), (3, "Thanks <users/1005>, that was fast.")],
    &[(1, "Welcome back <users/1004>! Hope the trip was good.")],
    &[(4, "Thank you, it was. Brought back far too much coffee.")],
    &[(2, "Quick note: I will be out Friday afternoon for a dentist appointment.")],
    &[(1, "We closed the Series A paperwork this morning. More details at Thursday's all hands, but I wanted you to hear it here first."), (3, "Congratulations everyone 🎉"), (4, "Huge. Well done <users/1001>."), (5, "Does this mean we get a second staging cluster?"), (1, "It means we get _one_ staging cluster that does not fall over, Dev.")],
    &[(3, "Lunch at the Keralan place at 1? I need to be back by 2 for the design sync.")],
    &[(4, "Count me in.")],
    &[(2, "Same.")],
    &[(5, "The lift in block B is out again. Use the stairs or the other wing.")],
    &[(1, "Please update your timesheets before Friday, finance is closing the quarter.")],
    &[(4, "Posted the Q4 roadmap draft here: https://docs.prism.test/roadmap-q4. Comments open until Monday.")],
    &[(2, "Read through the roadmap, left two comments on the _tiling_ section.")],
    &[(3, "Is the all hands recorded? I am at the dentist that hour.")],
    &[(1, "Yes, it will be in the shared drive by evening.")],
    &[(5, "PSA: if your laptop asks to install the new MDM profile, say yes. It is ours.")],
    &[(4, "Who owns the demo account for Kaveri? The password in the vault does not work."), (2, "I rotated it yesterday after the audit. Updated entry is under `demo/kaveri-2`."), (4, "Found it, thanks.")],
    &[(3, "Weekly dev metrics are up on the board. CI time is down to **6m40s** median.")],
    &[(1, "Nice work on the CI time, that was a long time coming.")],
    &[(2, "Heads up that I pushed a change to the shared `.editorconfig`. Tabs are gone, do not fight it.")],
    &[(5, "Tabs were never there to begin with 😄")],
    &[(4, "The new onboarding deck is in Drive, feedback welcome before I send it to the two new hires.")],
    &[(1, "Interview loop for the SRE role is Tuesday. <users/1005> you are on the systems round.")],
    &[(5, "On it. I will reuse the incident scenario from last time with a different failure.")],
    &[(3, "Trying the new `cargo nextest` runner locally. 40% faster on the proto crate.")],
    &[(2, "Reminder: retro board is open, add your cards before 3pm.")],
    &[(4, "Team photo at 5 by the terrace. Attendance optional, smiling mandatory.")],
    &[(1, "We have a visitor from the Pune office tomorrow, please clear the small meeting room.")],
    &[(5, "Backup restore drill passed. 11 minutes from snapshot to a working database.")],
    &[(2, "Does anyone have the Figma link for the empty states? The one in the ticket is dead.")],
    &[(4, "https://figma.com/file/prism-empty-states, I re-shared it with the team.")],
    &[(3, "Thanks, that works now.")],
    &[(1, "Friday demo order: Priya (ingest), Rahul (sidebar), Dev (deploy pipeline), Meera (research readout).")],
    &[(2, "Can I go second? My demo depends on a dataset that finishes loading around 4:15.")],
    &[(1, "Sure, swap with Rahul.")],
    &[(3, "Fine by me.")],
    &[(5, "Staging will be down 7 to 7:30 tonight for the Postgres major upgrade.")],
    &[(4, "I am collecting questions for the customer advisory board. Drop them in the doc by Wednesday.")],
    &[(2, "Added mine about export formats.")],
    &[(1, "Good discussion today. Summary: we keep the monthly cadence, we drop the Tuesday sync.")],
    &[(3, "Happy with that.")],
    &[(5, "The printer on floor 2 is now `print-2f`, the old one is retired.")],
    &[(4, "Lunch and learn Thursday: Rahul on rendering 100k map features without crying.")],
    &[(3, "I promise at least one slide of crying.")],
    &[(2, "Deploy of ingest v2.3 went out clean. Release notes are in #engineering.")],
    &[(1, "Thank you all for a solid week. Logging off early, see you Monday.")],
    &[(4, "Have a good weekend everyone.")],
    &[(5, "Pager is with me this weekend. Please do not deploy anything exciting.")],
    &[(2, "No promises.")],
    &[(3, "Monday morning question: who moved the retro board to a new column layout?"), (4, "Me. The old one had five columns and we only ever used three."), (3, "Fair. I like it.")],
    &[(1, "Quarterly planning kicks off next week. Meera is running the first session.")],
    &[(4, "Agenda will be out tomorrow. Bring your pet peeves.")],
    &[(5, "Reminder that the VPN cert expires on the 20th. Pull the new profile from the wiki.")],
    &[(2, "Done.")],
    &[(3, "Done as well.")],
    &[(1, "If anyone needs me I am in the small room on calls until 3.")],
];

const ENGINEERING: &[Conv] = &[
    &[(2, "Morning. Deploying `ingest` v2.3.0 to staging in ten minutes, shout if you need a hold.")],
    &[(5, "Heads up: staging Postgres is at **87% disk**. I will grow the volume tonight."), (2, "Thanks <users/1005>. Is that the WAL again?"), (5, "Yes. `archive_command` has been failing silently since Tuesday."), (1, "Can we alert on archive failures? That is twice this month."), (5, "Adding a check to the exporter now, PR in an hour.")],
    &[(3, "PR for the virtualised sidebar is up: https://git.prism.test/atlas/pull/412. It is big, sorry.")],
    &[(1, "I will take the sidebar review after lunch.")],
    &[(2, "The tile cache hit rate dropped to 71% after the deploy. Looking.")],
    &[(2, "Found it. We changed the quantisation of zoom levels and every key missed. Rolling back the key format.")],
    &[(5, "Fargate task `atlas-etl-rainfall` has been restarting for the last hour. Exit code 137, so memory."), (2, "The new district polygons are 4x larger. Bump the task to 4 GB?"), (5, "Bumped to 4 GB and it finished in 6 minutes. We should stream those features instead of loading them all."), (1, "Agreed, file it as tech debt for the next sprint.")],
    &[(3, "Is `formattedText` ever set when we create messages through the API, or only `text`?")],
    &[(2, "Only `text` on create. `formattedText` is output only.")],
    &[(1, "Design review notes from this morning are in the doc. The short version: we go with inline threads.")],
    &[(5, "New CI runners are live. Builds should be about 30% faster, tell me if anything regressed.")],
    &[(3, "Confirmed, the desktop build went from 9 minutes to 6.")],
    &[(2, "p95 for `/v1/features` is still 840 ms on the big states. Chart from this morning attached. [[img:latency]]"), (1, "Is that the GeoJSON serialisation or the query?"), (2, "Serialisation. The query is 90 ms. We build the whole string in memory."), (3, "Stream it with `axum::body::Body::from_stream`, we did that for exports."), (2, "Trying that now.")],
    &[(2, "Streaming brought p95 down to **310 ms**. PR 418.")],
    &[(1, "Lovely. Merge when CI is green.")],
    &[(5, "Reminder: do not use `latest` tags in the compose file. Pin everything, including Postgres.")],
    &[(3, "Guilty. Fixed in the next push.")],
    &[(2, "Question for <users/1001>: do we keep the legacy `/v0` endpoints for Kaveri or finally remove them?"), (1, "Keep them until their contract renewal in January, then remove with a two-week notice."), (2, "Noted. Adding a deprecation header now so their logs show it.")],
    &[(3, "Anyone know why `cargo clippy` complains about `needless_return` only on CI and not locally?")],
    &[(5, "Different toolchain. CI pinned 1.90, your laptop is on 1.91. I will bump CI.")],
    &[(3, "That explains it, thanks.")],
    &[(1, "Incident write-up for Monday's outage is ready for review: https://docs.prism.test/incidents/2026-11-02")],
    &[(2, "Read it. One nit: the timeline says 14:07 but the alert fired at 14:03.")],
    &[(1, "Fixed.")],
    &[(5, "Rotated the S3 access keys for the tile bucket. Nothing should have noticed; tell me if something did.")],
    &[(2, "Ingest noticed nothing. Good.")],
    &[(3, "Small thing: the dark theme map labels are unreadable at zoom 5. Filed as ATL-233.")],
    &[(1, "Quarterly dependency bump: `axum` 0.8, `tokio` 1.49, `reqwest` 0.12. All green locally.")],
    &[(2, "Here is the retry policy I am proposing for the Chat API client:\n```rust\nlet backoff = ExponentialBackoff::builder()\n    .base(Duration::from_millis(250))\n    .max_retries(5)\n    .retry_on(|s| s == 429 || s.is_server_error())\n    .build();\n```\nThoughts?"), (1, "Looks right. Also honour `Retry-After` when present, Google sends it on 429."), (3, "And cap the total wait at 30 s so the UI does not hang."), (2, "Both added.")],
    &[(5, "The staging cluster is now on Postgres 17. Vacuum is noticeably faster.")],
    &[(3, "Migration `0042_thread_index` ran in 2 seconds on staging, should be fine on prod.")],
    &[(2, "Please review PR 421 (search index rebuild) when you have a moment, it blocks the FTS work.")],
    &[(1, "Done, one comment about the tokenizer.")],
    &[(3, "What is our policy on `unwrap()` in handlers? I see both styles in the relay.")],
    &[(1, "No `unwrap()` on anything that comes from the network. Internal invariants are fine with a comment.")],
    &[(5, "Deployed the exporter change. Archive failures now page after 3 misses.")],
    &[(2, "Thanks. That would have saved us Tuesday.")],
    &[(3, "`pnpm install` is broken on a clean checkout, the lockfile references a tarball that moved."), (2, "Was that the `@tauri-apps/cli` nightly?"), (3, "Yes. Pinned to 2.2.1 and it is fine now.")],
    &[(1, "Code freeze for v2.4 is Thursday noon. Anything not merged by then goes to 2.5.")],
    &[(2, "The export job for Maharashtra ran out of disk on the worker. Using a 50 GB scratch volume from now on.")],
    &[(5, "Scratch volumes are ephemeral, please make sure the job cleans up on failure too.")],
    &[(2, "It does now.")],
    &[(3, "The sidebar PR is merged. Scrolling 2,000 spaces at 60 fps on my laptop.")],
    &[(1, "Great. Ship it to staging and let Meera click around.")],
    &[(4, "Clicking around. First impression: *fast*. Second impression: the unread dot is one pixel off.")],
    &[(3, "One pixel is a feature request, not a bug 😄")],
    &[(5, "Monitoring: added a dashboard for tile cache hit rate per zoom. Link in the runbook.")],
    &[(2, "> We build the whole string in memory.\n\nQuoting myself from Tuesday so nobody repeats it in the export path.")],
    &[(1, "Reviewing the OpenAPI diff for 2.4. Two breaking changes, both intentional, both documented.")],
    &[(3, "Flaky test alert: `relay::fanout::typing_ttl` fails one run in twenty on CI."), (2, "It sleeps for exactly the TTL. Use a tokio paused clock instead."), (3, "Switched to `tokio::time::pause()`. 200 runs green.")],
    &[(5, "Reminder that the on-call handover doc needs your notes by 5.")],
    &[(2, "Added mine. Nothing open except the disk alert tuning.")],
    &[(1, "Branch protection on `main` now requires one review and green CI. No more direct pushes.")],
    &[(3, "About time.")],
    &[(2, "The Workspace Events subscription renews every 4 hours now instead of 24. Fewer gaps after laptop sleep.")],
    &[(5, "Container image size is down from 1.2 GB to 180 MB with the distroless base.")],
    &[(1, "That is a big win for cold starts. Nice.")],
    &[(3, "I wrote up the markdown rendering rules we agreed on: https://docs.prism.test/markdown-rules")],
    &[(4, "Thanks <users/1003>, I linked it from the design system page.")],
    &[(2, "`cargo test` on the proto crate is 0.4 s. I love small crates.")],
    &[(5, "Pinned `rustls` to 0.23.19 after the advisory. Rebuild your images.")],
    &[(3, "Rebuilt.")],
    &[(1, "Please keep PR descriptions to what changed and why. The _how_ is in the diff.")],
    &[(2, "The ingest dashboard shows a gap between 02:00 and 02:40 last night. Scheduled job overlapped the backup.")],
    &[(5, "Moved the backup to 01:00. Should not overlap again.")],
    &[(3, "Added keyboard shortcuts to the message list: `j`/`k` to move, `r` to reply, `e` to edit.")],
    &[(4, "Can `Esc` close the thread panel too?")],
    &[(3, "It does now.")],
    &[(1, "Code freeze is in effect. Thank you for a clean week.")],
    &[(2, "Release notes draft for 2.4 is in the wiki. Add your items by tomorrow.")],
    &[(5, "The prod deploy of 2.4 is scheduled for Tuesday 10:00 IST. Rollback plan is in the same doc.")],
    &[(3, "I will be around for the deploy.")],
    &[(2, "Me too.")],
    &[(1, "Looking at the search latency numbers. Substring over 300 messages is fine, FTS can wait for v2.5.")],
    &[(3, "Agreed, the local cache is instant anyway.")],
    &[(5, "Updated the runbook with the new alert thresholds. Please skim it before your next on-call.")],
    &[(2, "Skimmed. Looks good.")],
    &[(1, "Last call for 2.4 release notes.")],
    &[(3, "Added the sidebar and shortcuts items.")],
    &[(2, "Added ingest and export items.")],
    &[(5, "Infra items in. Done.")],
];

const DESIGN: &[Conv] = &[
    &[(4, "New sidebar mock is up, with the compact density option we talked about. [[img:sidebar]]"), (1, "The compact density looks right. Can the unread count sit on the right edge?"), (4, "Yes, moving it. The avatar column stays 28px."), (2, "What happens to very long space names?"), (4, "Ellipsis after 22 characters, full name in the tooltip.")],
    &[(4, "Design tokens v3 are exported: `--glass-bg`, `--glass-border`, `--accent-500` and friends.")],
    &[(2, "Pulled them into the web view. Dark mode looks much better with the new border alpha.")],
    &[(1, "Can we get a token for the thread panel width? It is hardcoded to 360 in two places.")],
    &[(4, "Added `--thread-panel-w: 380px`. 360 was too tight for code blocks.")],
    &[(4, "Empty state copy for a space with no messages: _'Nothing here yet. Say hello.'_ Thoughts?")],
    &[(2, "I like it. Short.")],
    &[(1, "Works for me.")],
    &[(4, "Research readout from the five customer interviews is in the doc. Headline: people want **search that works**.")],
    &[(1, "Search is on the 2.5 list. Good to have the evidence.")],
    &[(4, "Message hover actions: react, reply, more. Three icons max, the rest in the menu."), (2, "Can edit be a top-level icon for your own messages?"), (4, "Only on hover of your own messages, yes. Four icons there, three on others."), (1, "Good compromise.")],
    &[(4, "Updated the reaction picker. Recent emoji row on top, search below.")],
    &[(2, "The picker closes when I click a reaction. Is that intended? It could stay open with shift.")],
    &[(4, "Intended for click, shift-click keeps it open. Added to the spec.")],
    &[(1, "Typing indicator placement: below the composer, not above the last message. Decided.")],
    &[(4, "Agreed, it stops the message list jumping.")],
    &[(4, "Avatar fallback is initials on a tinted circle. Two letters, 600 weight.")],
    &[(2, "Matches what the twin serves, nice.")],
    &[(4, "Link previews: we show title and domain only, no image, for v2.4.")],
    &[(1, "Fine. Images in 2.5 if people ask.")],
    &[(4, "Day dividers use the short date: _Today_, _Yesterday_, then `Mon 3 Nov`.")],
    &[(2, "Implemented. Looks clean.")],
    &[(4, "Presence dot colours: green active, amber away, red dnd, grey offline. Same as the tokens.")],
    &[(1, "Can dnd have a small minus sign? The colour alone is not accessible.")],
    &[(4, "Yes, adding the glyph.")],
    &[(4, "Onboarding flow is down to three screens from five. Sign in, pick spaces, done.")],
    &[(2, "Three is good. The old step 4 confused everyone in testing.")],
    &[(4, "Thread panel header shows the first message with its sender and time, then replies below a divider.")],
    &[(1, "And the reply count in the space list, like `3 replies`.")],
    &[(4, "Yes, with the last reply time next to it.")],
    &[(4, "Composer: markdown preview toggle lives in the toolbar, off by default."), (2, "Keyboard shortcut?"), (4, "`Cmd+Shift+P`. Added to the shortcut sheet.")],
    &[(4, "Accessibility pass done on the sidebar. Two contrast fixes, one focus ring fix, all in the tokens.")],
    &[(1, "Thank you. Let us do the message list next week.")],
    &[(4, "Custom status: emoji plus 40 characters, optional expiry. Mirrors the relay message.")],
    &[(2, "40 is tight but fine.")],
    &[(4, "Notification sound is the soft two-tone from the options doc. Louder one rejected by everyone.")],
    &[(1, "Correct decision.")],
    &[(4, "Settings page sketch: Appearance, Notifications, Accounts, Advanced. Nothing else.")],
    &[(2, "Where does the relay URL go?")],
    &[(4, "Advanced. Nobody but us should touch it.")],
    &[(4, "I will be in the Kaveri call tomorrow showing the sidebar. Wish me luck.")],
    &[(1, "You will not need it.")],
    &[(4, "Kaveri loved the density toggle. They asked for a wider thread panel, which we already did.")],
    &[(2, "Perfect timing then.")],
];

const RANDOM: &[Conv] = &[
    &[(3, "New keyboard arrived. It is loud. Apologies in advance.")],
    &[(5, "Louder than Priya's? Impossible.")],
    &[(2, "My keyboard is _vintage_, not loud.")],
    &[(
        4,
        "Anyone want the extra tomato plants from my balcony? I have far too many.",
    )],
    &[(1, "Yes please, two.")],
    &[(
        3,
        "Weekend plan: finally finishing the hike that rained out last month.",
    )],
    &[(
        5,
        "Watched the new sci-fi thing everyone talks about. It is fine. Just fine.",
    )],
    &[(2, "Strong words.")],
    &[(
        4,
        "Book club pick for the month is _Project Hail Mary_. Thursday evening, terrace.",
    )],
    &[(3, "Read it twice already, so I am in.")],
    &[(1, "Haven't read it. Also in.")],
    &[(
        5,
        "The cafe downstairs now has filter coffee. Civilisation has arrived.",
    )],
    &[(2, "Trying it now. Report to follow.")],
    &[(2, "Report: acceptable.")],
    &[(3, "Who left a half-eaten samosa on the standing desk?")],
    &[(5, "Not me. Probably Dev. Oh wait.")],
    &[(
        4,
        "Cricket on Saturday at the ground near the office, 7am. Yes, 7am.",
    )],
    &[(1, "7am is a crime but I will be there.")],
    &[(3, "I will bring the stumps.")],
    &[(2, "I will bring the excuses.")],
    &[
        (5, "We won. Barely. Rahul bowled like a man possessed."),
        (3, "Possessed by what exactly?"),
        (5, "Something that cannot bat, apparently."),
        (1, "Both true. Good game."),
    ],
    &[(
        4,
        "Found a great noodle place two streets down. Lunch tomorrow?",
    )],
    &[(2, "In.")],
    &[(3, "In.")],
    &[(1, "In, if the 1pm call ends on time.")],
    &[(5, "It will not, but go without me and bring me something.")],
    &[(4, "Noodles were excellent. Dev, yours is on your desk.")],
    &[(5, "You are a hero.")],
    &[(
        3,
        "Ran 10k this morning before standup. Standup was the hard part.",
    )],
    &[(
        2,
        "Does anyone have a spare USB-C to HDMI? Mine died mid-demo.",
    )],
    &[(1, "In the drawer by the printer, there are three.")],
    &[(
        4,
        "Friday playlist is open for additions. Keep it under 120 bpm, we have rules.",
    )],
    &[(3, "Added two tracks that are exactly 119 bpm.")],
    &[(5, "The rules are a suggestion. Added something at 160.")],
    &[(4, "Removed. The rules are the rules.")],
    &[(
        2,
        "Best lunch spot near the Pune office? I am there next week.",
    )],
    &[(1, "Ask for Sunil at the canteen and say I sent you.")],
    &[(
        3,
        "Long weekend plans: nothing. Absolutely nothing. It will be glorious.",
    )],
    &[(5, "Same. Pager permitting.")],
    &[(
        4,
        "Reminder that the plant by the window needs water on Fridays. It is looking at you.",
    )],
];

const ANNOUNCEMENTS: &[Conv] = &[
    &[(1, "**All hands Thursday 5pm.** Agenda: funding update, Q4 roadmap, hiring plan. Recording will be posted.")],
    &[(4, "Q4 roadmap is final: https://docs.prism.test/roadmap-q4. Thank you for the comments, most made it in.")],
    &[(1, "Two new hires start on the 17th: an SRE and a product designer. Please make them welcome.")],
    &[(4, "Holiday calendar for next year is published. Floating holidays stay at three.")],
    &[(1, "v2.4 ships to production Tuesday 10:00 IST. Customer comms go out Monday evening.")],
    &[(4, "Customer advisory board meets on the 21st. Questions in the doc by Wednesday please.")],
    &[(1, "The office is closed Friday the 14th for the building electrical work. Work from home.")],
    &[(4, "Expense reports for this month are due by the 28th.")],
];

const DM_ARUN_PRIYA: &[Conv] = &[
    &[(1, "Got a minute? Want to talk about the ingest ownership before the planning session.")],
    &[(2, "Sure, free until 11.")],
    &[(1, "I think you should own ingest end to end, including the ETL schedules Dev runs today.")],
    &[(2, "Happy to, as long as the Fargate budget comes with it.")],
    &[(1, "It does. I will tell Dev this afternoon.")],
    &[(2, "Thanks. Can we also talk about the Kaveri contract? Their `/v0` usage is still 40% of traffic.")],
    &[(1, "January. We agreed to keep it until their renewal.")],
    &[(2, "Right, I remember now. Deprecation header is in place anyway.")],
    &[(2, "The latency fix is merged. Want to see the before and after graph?")],
    &[(1, "Yes, drop it in #engineering so everyone sees it.")],
    &[(2, "Done.")],
    &[(1, "Good work this week. The streaming change was the right call.")],
    &[(2, "Thanks. Rahul's idea really, I just typed it.")],
    &[(1, "Typing counts.")],
    &[(2, "Are you around Friday afternoon? I have the dentist at 3.")],
    &[(1, "Yes, I will cover the on-call channel.")],
    &[(2, "Appreciated.")],
    &[(1, "One more thing: can you mentor the new SRE on the ingest side for the first two weeks?")],
    &[(2, "Of course. I will put together a short reading list.")],
    &[(1, "Perfect.")],
    &[(2, "Quick one: the retry policy PR, do you want `Retry-After` parsed as seconds only or also as a date?")],
    &[(1, "Both, Google uses seconds but let us not assume.")],
    &[(2, "Both it is.")],
    &[(1, "Planning session moved to Tuesday 2pm. Meera's call.")],
    &[(2, "Works for me.")],
    &[(2, "Leaving now, dentist. Back online around 5 if anything burns.")],
    &[(1, "Nothing will burn. Go.")],
    &[(2, "Back. Nothing burned?")],
    &[(1, "Nothing burned. Dev deployed the exporter change and that was the whole afternoon.")],
    &[(2, "Quiet is good. See you Monday.")],
];

const GROUP_CORE: &[Conv] = &[
    &[(1, "Core group: let us use this for v2.4 release coordination so #engineering stays readable.")],
    &[(2, "Good idea.")],
    &[(3, "Agreed.")],
    &[(1, "Release checklist is in the wiki. Owners: Priya ingest, Rahul desktop, me the relay.")],
    &[(2, "Ingest items are green except the export cleanup, finishing today.")],
    &[(3, "Desktop: sidebar merged, shortcuts merged, the markdown preview is behind a flag.")],
    &[(1, "Relay: subscription renewal is in, dedup window is 60 s, tests green.")],
    &[(2, "Should the dedup key include the event type? A create then a delete within a minute would collide."), (1, "Good catch. Adding event type to the key."), (3, "And the `lastUpdateTime` for edits, which it already has."), (1, "Yes, key is type + name + lastUpdateTime now.")],
    &[(3, "Staging smoke test passed on both desktop instances. Typing indicators work across them.")],
    &[(2, "Same here with the browser build.")],
    &[(1, "Then we are go for Tuesday.")],
    &[(3, "What is the rollback trigger? Error rate or latency?")],
    &[(1, "Error rate above 1% for five minutes, or any data loss report. Latency we tolerate for an hour.")],
    &[(2, "Written into the plan.")],
    &[(1, "Tuesday 10:00 then. I will start the deploy, Dev watches the dashboards.")],
    &[(3, "I will be on the desktop side checking the first sessions.")],
    &[(2, "I will watch ingest and the events pipeline.")],
    &[(1, "Deploy started.")],
    &[(2, "Ingest healthy. Events flowing.")],
    &[(3, "Desktop sessions connecting. No errors in the first 50.")],
    &[(1, "Relay memory steady at 90 MB. Looking good.")],
    &[(2, "30 minutes in, nothing to report.")],
    &[(1, "Calling it. v2.4 is live. Thank you both.")],
    &[(3, "🎉")],
    &[(2, "Now, about 2.5...")],
];

const SPACES: &[SpaceDef] = &[
    SpaceDef {
        id: "spaces/AAAAgeneral",
        display_name: Some("general"),
        space_type: "SPACE",
        description: "Company-wide chatter. Everyone is here.",
        members: &[1, 2, 3, 4, 5],
        managers: &[1],
        convs: GENERAL,
    },
    SpaceDef {
        id: "spaces/AAAAengineering",
        display_name: Some("engineering"),
        space_type: "SPACE",
        description: "Deploys, PRs, incidents and the occasional argument about tabs.",
        members: &[1, 2, 3, 5],
        managers: &[1],
        convs: ENGINEERING,
    },
    SpaceDef {
        id: "spaces/AAAAdesign",
        display_name: Some("design"),
        space_type: "SPACE",
        description: "Mocks, tokens, research readouts.",
        members: &[1, 2, 4],
        managers: &[4],
        convs: DESIGN,
    },
    SpaceDef {
        id: "spaces/AAAArandom",
        display_name: Some("random"),
        space_type: "SPACE",
        description: "Not work.",
        members: &[1, 2, 3, 4, 5],
        managers: &[1],
        convs: RANDOM,
    },
    SpaceDef {
        id: "spaces/AAAAannounce",
        display_name: Some("announcements"),
        space_type: "SPACE",
        description: "Read-mostly. Managers post, everyone reads.",
        members: &[1, 2, 3, 4, 5],
        managers: &[1, 4],
        convs: ANNOUNCEMENTS,
    },
    SpaceDef {
        id: "spaces/AAAAdm1001x1002",
        display_name: None,
        space_type: "DIRECT_MESSAGE",
        description: "",
        members: &[1, 2],
        managers: &[],
        convs: DM_ARUN_PRIYA,
    },
    SpaceDef {
        id: "spaces/AAAAgrpcore",
        display_name: None,
        space_type: "GROUP_CHAT",
        description: "",
        members: &[1, 2, 3],
        managers: &[],
        convs: GROUP_CORE,
    },
];

const EMOJI_WEIGHTED: &[&str] = &[
    "👍", "👍", "👍", "👍", "🎉", "🎉", "👀", "👀", "❤️", "❤️", "😂",
];

pub fn user_name(idx: u8) -> String {
    format!("users/{}", USERS[(idx - 1) as usize].0)
}

/// Build the whole store. `now` anchors the seven-day window.
pub fn seed(config: &Config, now: DateTime<Utc>) -> Store {
    let mut rng = DetRng::new(0x5EED_2026_1106);
    let mut store = Store {
        public_base: config.public_base.clone(),
        token_ttl_secs: config.token_ttl_secs,
        ..Default::default()
    };

    for (id, display, local) in USERS {
        let name = format!("users/{id}");
        store.users.insert(
            name.clone(),
            User {
                name,
                display_name: Some((*display).to_string()),
                domain_id: Some("prism-test".into()),
                user_type: Some("HUMAN".into()),
                is_anonymous: Some(false),
                avatar_url: Some(store.avatar_url(id)),
                email: Some(format!("{local}@{DOMAIN}")),
            },
        );
    }

    let created = fmt_time(now - Duration::days(60));
    for def in SPACES {
        store.new_space(
            def.id,
            def.space_type,
            def.display_name,
            def.description,
            &created,
        );
        for m in def.members {
            let role = if def.managers.contains(m) {
                "ROLE_MANAGER"
            } else {
                "ROLE_MEMBER"
            };
            store.add_member(def.id, &user_name(*m), role, &created);
        }
    }

    let timeline = Timeline::new(now);
    for def in SPACES {
        seed_messages(&mut store, &mut rng, def, &timeline, now);
    }
    seed_reactions(&mut store, &mut rng);
    seed_read_states(&mut store, &mut rng, now);
    seed_tasks(&mut store, now);
    seed_calendar(&mut store, now);
    store
}

/// The Standup's Meet link, fixed so scripts can find it.
pub const STANDUP_MEET: &str = "https://meet.google.com/abc-defg-hij";

/// `date` at `h:m` in the twin's local zone (the earlier instant on a DST fold).
fn local_at(date: NaiveDate, h: u32, m: u32) -> DateTime<Local> {
    let naive = date.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("valid time"));
    Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive))
}

fn timed(start: DateTime<Local>, end: DateTime<Local>) -> (EventTime, EventTime) {
    let at = |t: DateTime<Local>| EventTime {
        date_time: Some(Store::cal_time(t)),
        date: None,
        time_zone: None,
    };
    (at(start), at(end))
}

/// An attendee by seed user index; `self` is never stored, the twin sets it per caller.
fn attendee(idx: u8, status: &str, organizer: bool) -> Attendee {
    let (_, display, local) = USERS[(idx - 1) as usize];
    Attendee {
        email: format!("{local}@{DOMAIN}"),
        display_name: Some(display.to_string()),
        response_status: Some(status.to_string()),
        is_self: false,
        organizer,
        optional: false,
    }
}

/// Four events relative to `now`, in the twin's local zone: on `primary`, "Standup"
/// today 10:00 to 10:15 with a Meet link, "Design review" two hours from now for 45
/// minutes with a (non-Meet) video entry point, "Relay deploy window" tomorrow 15:00
/// to 16:00 with no attendees; on `team@example.test`, "All hands" in three days 11:00
/// to 12:00. Ids `ev-1` to `ev-4` in that order.
fn seed_calendar(store: &mut Store, now: DateTime<Utc>) {
    let local_now = now.with_timezone(&Local);
    let today = local_now.date_naive();

    let (start, end) = timed(local_at(today, 10, 0), local_at(today, 10, 15));
    let organizer = attendee(1, Attendee::ACCEPTED, true);
    store.new_cal_event(
        CAL_PRIMARY,
        CalEvent {
            summary: Some("Standup".into()),
            description: Some("Fifteen minutes: what moved, what is stuck.".into()),
            hangout_link: Some(STANDUP_MEET.into()),
            conference_data: Some(ConferenceData {
                entry_points: vec![EntryPoint {
                    entry_point_type: Some("video".into()),
                    uri: Some(STANDUP_MEET.into()),
                    label: Some(STANDUP_MEET.trim_start_matches("https://").into()),
                }],
            }),
            start,
            end,
            attendees: vec![
                organizer.clone(),
                attendee(2, Attendee::ACCEPTED, false),
                attendee(3, Attendee::NEEDS_ACTION, false),
            ],
            organizer: Some(organizer),
            ..Default::default()
        },
        now,
    );

    let review_start = (local_now + Duration::hours(2))
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(local_now + Duration::hours(2));
    let (start, end) = timed(review_start, review_start + Duration::minutes(45));
    let organizer = attendee(1, Attendee::ACCEPTED, true);
    store.new_cal_event(
        CAL_PRIMARY,
        CalEvent {
            summary: Some("Design review".into()),
            description: Some("Sidebar density and the thread panel.".into()),
            location: Some("Room 2B".into()),
            conference_data: Some(ConferenceData {
                entry_points: vec![EntryPoint {
                    entry_point_type: Some("video".into()),
                    uri: Some("https://zoom.example.test/j/8675309".into()),
                    label: Some("zoom.example.test/j/8675309".into()),
                }],
            }),
            start,
            end,
            attendees: vec![
                organizer.clone(),
                attendee(2, Attendee::NEEDS_ACTION, false),
                attendee(4, Attendee::ACCEPTED, false),
            ],
            organizer: Some(organizer),
            ..Default::default()
        },
        now,
    );

    let tomorrow = today + Duration::days(1);
    let (start, end) = timed(local_at(tomorrow, 15, 0), local_at(tomorrow, 16, 0));
    store.new_cal_event(
        CAL_PRIMARY,
        CalEvent {
            summary: Some("Relay deploy window".into()),
            description: Some("relay 0.1.0 to prod; the twin stays up.".into()),
            start,
            end,
            organizer: Some(attendee(1, Attendee::ACCEPTED, true)),
            ..Default::default()
        },
        now,
    );

    let in_three_days = today + Duration::days(3);
    let (start, end) = timed(
        local_at(in_three_days, 11, 0),
        local_at(in_three_days, 12, 0),
    );
    let organizer = attendee(4, Attendee::ACCEPTED, true);
    store.new_cal_event(
        CAL_TEAM,
        CalEvent {
            summary: Some("All hands".into()),
            description: Some("Quarter in review, then questions.".into()),
            location: Some("Town hall".into()),
            start,
            end,
            attendees: vec![
                organizer.clone(),
                attendee(1, Attendee::NEEDS_ACTION, false),
                attendee(2, Attendee::NEEDS_ACTION, false),
                attendee(3, Attendee::NEEDS_ACTION, false),
                attendee(5, Attendee::NEEDS_ACTION, false),
            ],
            organizer: Some(organizer),
            ..Default::default()
        },
        now,
    );
}

/// Two tasks in the default list: one with notes, one due tomorrow at midnight UTC.
fn seed_tasks(store: &mut Store, now: DateTime<Utc>) {
    store.new_task(
        "Review the relay reconnect PR",
        Some("spaces/AAAAgeneral thread from Priya"),
        None,
        now,
    );
    let tomorrow = (now + Duration::days(1))
        .date_naive()
        .and_time(NaiveTime::MIN)
        .and_utc();
    store.new_task(
        "Send the design spec to Priya",
        None,
        Some(&Store::task_time(tomorrow)),
        now,
    );
}

/// Working-hours windows (09:00–21:00 IST) over the last seven days, so a
/// fraction in `0..1` maps to a believable timestamp in order.
struct Timeline {
    windows: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    total_secs: f64,
}

impl Timeline {
    fn new(now: DateTime<Utc>) -> Self {
        let latest = now - Duration::seconds(90);
        let mut windows = Vec::new();
        let start_day = (now - Duration::days(6)).date_naive();
        for d in 0..7 {
            let date = start_day + Duration::days(d);
            // 09:00 IST = 03:30 UTC, 21:00 IST = 15:30 UTC
            let open = date
                .and_time(NaiveTime::from_hms_opt(3, 30, 0).unwrap())
                .and_utc();
            let close = date
                .and_time(NaiveTime::from_hms_opt(15, 30, 0).unwrap())
                .and_utc();
            let close = close.min(latest);
            if close > open {
                windows.push((open, close));
            }
        }
        // Today before office hours: still give the last stretch some recent traffic.
        if windows
            .last()
            .map(|w| w.1 < latest - Duration::hours(3))
            .unwrap_or(true)
        {
            windows.push((latest - Duration::hours(2), latest));
        }
        let total_secs = windows
            .iter()
            .map(|(a, b)| (*b - *a).num_seconds() as f64)
            .sum();
        Self {
            windows,
            total_secs,
        }
    }

    fn at(&self, fraction: f64) -> DateTime<Utc> {
        let mut x = fraction.clamp(0.0, 0.999_999) * self.total_secs;
        for (open, close) in &self.windows {
            let len = (*close - *open).num_seconds() as f64;
            if x < len {
                return *open + Duration::seconds(x as i64);
            }
            x -= len;
        }
        self.windows.last().map(|w| w.1).unwrap_or_default()
    }
}

fn seed_messages(
    store: &mut Store,
    rng: &mut DetRng,
    def: &SpaceDef,
    timeline: &Timeline,
    now: DateTime<Utc>,
) {
    let n = def.convs.len() as f64;
    let latest = now - Duration::seconds(60);
    let mut last_time = None;
    for (i, conv) in def.convs.iter().enumerate() {
        let head_time = timeline.at((i as f64 + rng.f64() * 0.8) / n)
            + Duration::microseconds(rng.below(1_000_000) as i64);
        let thread_id = rng.id();
        let thread_name = format!("{}/threads/{thread_id}", def.id);
        let mut t = head_time;
        for (j, (sender, text)) in conv.iter().enumerate() {
            if j > 0 {
                t = (t
                    + Duration::minutes(2 + rng.below(38) as i64)
                    + Duration::milliseconds(rng.below(1000) as i64))
                .min(latest);
            }
            let msg_id = if j == 0 { thread_id.clone() } else { rng.id() };
            let name = format!("{}/messages/{thread_id}.{msg_id}", def.id);
            let (text, image) = split_image_marker(text);
            let create_time = fmt_time(t);
            let mut msg = Message {
                name: name.clone(),
                sender: store.users.get(&user_name(*sender)).cloned(),
                create_time: Some(create_time.clone()),
                text: Some(text.clone()),
                thread: Some(Thread {
                    name: Some(thread_name.clone()),
                    thread_key: None,
                }),
                thread_reply: Some(j > 0),
                space: Some(Space {
                    name: def.id.to_string(),
                    ..Default::default()
                }),
                annotations: store.annotate(&text),
                ..Default::default()
            };
            if let Some(key) = image {
                msg.attachment = vec![seed_attachment(store, def.id, &name, key)];
            }
            store.messages.insert(name, msg);
            last_time = Some(create_time);
        }
    }
    if let Some(t) = last_time {
        store.touch_space(def.id, &t);
    }
}

fn split_image_marker(text: &str) -> (String, Option<&'static str>) {
    if let Some(idx) = text.find("[[img:") {
        let key = &text[idx + 6..text.len() - 2];
        let key = match key {
            "latency" => Some("latency"),
            "sidebar" => Some("sidebar"),
            _ => None,
        };
        return (text[..idx].trim_end().to_string(), key);
    }
    (text.to_string(), None)
}

fn seed_attachment(store: &mut Store, space: &str, message: &str, key: &'static str) -> Attachment {
    let (filename, svg) = match key {
        "latency" => ("features-p95-latency.svg", latency_chart_svg()),
        _ => ("sidebar-compact-mock.svg", sidebar_mock_svg()),
    };
    let resource = format!("{space}/attachments/seed-{key}");
    store.blobs.insert(
        resource.clone(),
        Blob {
            space: space.to_string(),
            filename: filename.to_string(),
            content_type: "image/svg+xml".into(),
            bytes: svg.into_bytes(),
        },
    );
    attachment_for(
        &store.public_base,
        message,
        &resource,
        filename,
        "image/svg+xml",
        &format!("seed-{key}"),
    )
}

/// The `Attachment` resource a message carries for a stored blob.
pub fn attachment_for(
    public_base: &str,
    message: &str,
    resource: &str,
    filename: &str,
    content_type: &str,
    id: &str,
) -> Attachment {
    let url = format!("{public_base}/v1/media/{resource}?alt=media");
    Attachment {
        name: Some(format!("{message}/attachments/{id}")),
        content_name: Some(filename.to_string()),
        content_type: Some(content_type.to_string()),
        attachment_data_ref: Some(AttachmentDataRef {
            resource_name: Some(resource.to_string()),
            attachment_upload_token: None,
        }),
        drive_data_ref: None,
        thumbnail_uri: Some(url.clone()),
        download_uri: Some(url),
        source: Some("UPLOADED_CONTENT".into()),
    }
}

fn seed_reactions(store: &mut Store, rng: &mut DetRng) {
    let names: Vec<String> = store.messages.keys().cloned().collect();
    let mut touched = Vec::new();
    for name in names {
        if !rng.chance(0.3) {
            continue;
        }
        let space = store.messages[&name].space_name().to_string();
        let members: Vec<String> = store
            .members_of(&space)
            .into_iter()
            .filter_map(|m| m.member.map(|u| u.name))
            .collect();
        let count = 1 + rng.below(3) as usize;
        for _ in 0..count {
            let user = rng.pick(&members).clone();
            let emoji = (*rng.pick(EMOJI_WEIGHTED)).to_string();
            let dup = store.reactions_of(&name).iter().any(|r| {
                r.user.as_ref().map(|u| u.name.as_str()) == Some(user.as_str())
                    && r.emoji.unicode.as_deref() == Some(emoji.as_str())
            });
            if dup {
                continue;
            }
            store.reactions.push(Reaction {
                name: Some(format!("{name}/reactions/{}", rng.id())),
                user: store.users.get(&user).cloned(),
                emoji: Emoji {
                    unicode: Some(emoji),
                    custom_emoji: None,
                },
            });
        }
        touched.push(name);
    }
    for name in touched {
        store.recompute_summaries(&name);
    }
}

fn seed_read_states(store: &mut Store, rng: &mut DetRng, now: DateTime<Utc>) {
    let keys: Vec<(String, String)> = store.read_states.keys().cloned().collect();
    for (user, space) in keys {
        let times: Vec<String> = store
            .messages_in(&space)
            .into_iter()
            .filter_map(|m| m.create_time.clone())
            .collect();
        let unread = rng.below(9) as usize;
        let last_read = if unread == 0 || times.len() <= unread {
            fmt_time(now)
        } else {
            times[times.len() - unread - 1].clone()
        };
        store.read_states.insert((user, space), last_read);
    }
}

/// SVG avatar: initials on a tinted circle.
pub fn avatar_svg(id: &str) -> Option<String> {
    let (idx, (_, display, _)) = USERS.iter().enumerate().find(|(_, u)| u.0 == id)?;
    let initials: String = display
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let color = AVATAR_COLORS[idx % AVATAR_COLORS.len()];
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"128\" viewBox=\"0 0 128 128\">\
<rect width=\"128\" height=\"128\" rx=\"64\" fill=\"{color}\"/>\
<text x=\"64\" y=\"64\" dy=\"0.36em\" text-anchor=\"middle\" font-family=\"Inter, Helvetica, Arial, sans-serif\" font-size=\"52\" font-weight=\"600\" fill=\"#ffffff\">{initials}</text>\
</svg>"
    ))
}

fn latency_chart_svg() -> String {
    let points = [840, 812, 790, 805, 760, 720, 690, 640, 520, 410, 350, 310];
    let w = 640.0;
    let h = 320.0;
    let step = (w - 80.0) / (points.len() as f64 - 1.0);
    let path: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = 50.0 + i as f64 * step;
            let y = h - 40.0 - (*p as f64 / 900.0) * (h - 80.0);
            format!("{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" })
        })
        .collect();
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\
<rect width=\"{w}\" height=\"{h}\" fill=\"#0f1220\"/>\
<text x=\"50\" y=\"28\" fill=\"#e6e8ef\" font-family=\"Inter, Helvetica, sans-serif\" font-size=\"16\" font-weight=\"600\">/v1/features p95 latency (ms), last 12 deploys</text>\
<line x1=\"50\" y1=\"{yb}\" x2=\"{xr}\" y2=\"{yb}\" stroke=\"#3a3f58\"/>\
<line x1=\"50\" y1=\"40\" x2=\"50\" y2=\"{yb}\" stroke=\"#3a3f58\"/>\
<text x=\"12\" y=\"44\" fill=\"#9aa0b5\" font-size=\"11\" font-family=\"Inter, Helvetica, sans-serif\">900</text>\
<text x=\"20\" y=\"{yb}\" fill=\"#9aa0b5\" font-size=\"11\" font-family=\"Inter, Helvetica, sans-serif\">0</text>\
<path d=\"{path}\" fill=\"none\" stroke=\"#5b6cff\" stroke-width=\"3\" stroke-linejoin=\"round\"/>\
<text x=\"{xr}\" y=\"{yl}\" fill=\"#5b6cff\" font-size=\"12\" text-anchor=\"end\" font-family=\"Inter, Helvetica, sans-serif\">310 ms after streaming</text>\
</svg>",
        yb = h - 40.0,
        xr = w - 30.0,
        yl = h - 40.0 - (310.0 / 900.0) * (h - 80.0) - 10.0,
        path = path.join(" ")
    )
}

fn sidebar_mock_svg() -> String {
    let rows = [
        "general",
        "engineering",
        "design",
        "random",
        "announcements",
    ];
    let mut items = String::new();
    for (i, r) in rows.iter().enumerate() {
        let y = 92 + i as i32 * 34;
        let weight = if i == 1 { "600" } else { "400" };
        items.push_str(&format!(
            "<text x=\"40\" y=\"{y}\" fill=\"#e6e8ef\" font-size=\"14\" font-weight=\"{weight}\" font-family=\"Inter, Helvetica, sans-serif\"># {r}</text>"
        ));
        if i == 1 {
            items.push_str(&format!(
                "<rect x=\"222\" y=\"{}\" width=\"22\" height=\"16\" rx=\"8\" fill=\"#5b6cff\"/><text x=\"233\" y=\"{}\" fill=\"#fff\" font-size=\"11\" text-anchor=\"middle\" font-family=\"Inter, Helvetica, sans-serif\">3</text>",
                y - 12, y
            ));
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"560\" height=\"300\" viewBox=\"0 0 560 300\">\
<rect width=\"560\" height=\"300\" fill=\"#14172a\"/>\
<rect x=\"16\" y=\"16\" width=\"248\" height=\"268\" rx=\"12\" fill=\"#1c2037\" stroke=\"#2d3350\"/>\
<text x=\"32\" y=\"48\" fill=\"#9aa0b5\" font-size=\"11\" letter-spacing=\"1\" font-family=\"Inter, Helvetica, sans-serif\">SPACES  ·  COMPACT</text>\
{items}\
<text x=\"300\" y=\"48\" fill=\"#e6e8ef\" font-size=\"15\" font-weight=\"600\" font-family=\"Inter, Helvetica, sans-serif\">Sidebar, compact density</text>\
<text x=\"300\" y=\"76\" fill=\"#9aa0b5\" font-size=\"12\" font-family=\"Inter, Helvetica, sans-serif\">Row height 34px, avatar column 28px,</text>\
<text x=\"300\" y=\"94\" fill=\"#9aa0b5\" font-size=\"12\" font-family=\"Inter, Helvetica, sans-serif\">unread badge right-aligned, 22 char ellipsis.</text>\
</svg>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_and_sized() {
        let cfg = Config::for_tests("http://127.0.0.1:9/x");
        let now = Utc::now();
        let a = seed(&cfg, now);
        let b = seed(&cfg, now);
        assert_eq!(a.users.len(), 5);
        assert_eq!(a.spaces.len(), 7);
        assert!(a.messages.len() >= 280, "got {}", a.messages.len());
        let names_a: Vec<&String> = a.messages.keys().collect();
        let names_b: Vec<&String> = b.messages.keys().collect();
        assert_eq!(names_a, names_b);
        let threads = a
            .messages
            .values()
            .filter(|m| m.thread_reply == Some(true))
            .filter_map(|m| m.thread.as_ref().and_then(|t| t.name.clone()))
            .collect::<std::collections::HashSet<_>>();
        assert!(threads.len() >= 14, "threads {}", threads.len());
        assert_eq!(
            a.messages
                .values()
                .filter(|m| !m.attachment.is_empty())
                .count(),
            2
        );
        assert!(a.reactions.len() > 50);
        assert!(a.messages.values().any(|m| !m.annotations.is_empty()));
        for m in a.messages.values() {
            let t = crate::util::parse_time(m.create_time.as_deref().unwrap()).unwrap();
            assert!(t <= now && t >= now - Duration::days(8));
        }
    }

    #[test]
    fn avatars_render() {
        assert!(avatar_svg("1001").unwrap().contains(">AJ<"));
        assert!(avatar_svg("9").is_none());
    }
}
