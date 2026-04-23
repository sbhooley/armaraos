# Reddit Marketing Campaign - Master Plan
## ArmaraOS & AI Native Lang Community Building

**Created:** 2026-04-22  
**Status:** Ready to Execute  
**Owner:** Steven Hooley (@sbhooley)

---

## 📚 Campaign Documents

This folder contains everything needed to successfully launch and maintain a Reddit presence for ArmaraOS and AI Native Lang without getting banned or having posts rejected.

### Core Documents

1. **[REDDIT_MARKETING_STRATEGY.md](./REDDIT_MARKETING_STRATEGY.md)** (110 pages)
   - Complete week-by-week strategy
   - Reddit's 2026 rules and requirements
   - Target subreddits and posting guidelines
   - Anti-spam techniques
   - Emergency procedures
   - Monthly content calendar

2. **[reddit_posts_ready_to_use.md](./reddit_posts_ready_to_use.md)** (Ready-to-post content)
   - 4 complete posts with titles and bodies
   - Pre-written comment responses
   - Timing guidelines
   - Post-publication checklists
   - Emergency scripts
   - Hacker News bonus post

3. **[REDDIT_QUICKSTART_GUIDE.md](./REDDIT_QUICKSTART_GUIDE.md)** (15-minute setup)
   - Account creation walkthrough
   - Week 1 daily checklist
   - Karma building strategies
   - Common mistakes to avoid
   - Quick reference card

---

## 🎯 Campaign Overview

### Goal
Build authentic Reddit community presence that drives organic traffic to ArmaraOS and AI Native Lang without triggering spam filters or community backlash.

### Timeline
- **Weeks 1-4:** Karma building (NO promotion)
- **Weeks 5-8:** Strategic value posts launch
- **Months 3-6:** Sustainable engagement
- **Month 6+:** Maintenance mode

### Success Metrics
- **Month 1:** 200+ karma, zero violations
- **Month 2:** First post 100-500 upvotes
- **Month 3:** 500+ karma, recognized contributor
- **Month 6:** 1000+ karma, AMA-ready, 15% traffic increase from Reddit

---

## 🚀 Quick Start (Choose Your Path)

### Option A: DIY Execution
1. Read **[REDDIT_QUICKSTART_GUIDE.md](./REDDIT_QUICKSTART_GUIDE.md)** (15 min)
2. Create account following guidelines
3. Week 1: Build karma (30 min/day)
4. Week 4: Review **[reddit_posts_ready_to_use.md](./reddit_posts_ready_to_use.md)**
5. Week 5: Post first value content

### Option B: Guided Execution (with Claude)
1. Ask Claude to create Reddit account
2. Claude drafts daily comments for review
3. Claude monitors karma progress
4. Claude schedules posts at optimal times
5. Claude helps with comment responses

### Option C: Automated (after Week 4)
1. Manual karma building (Weeks 1-4)
2. Enable ArmaraOS Reddit adapter (`crates/openfang-channels/src/reddit.rs`)
3. Automate monitoring and responses
4. Human approval for all posts

---

## 📋 The Strategy (TL;DR)

### Reddit's 2026 Rules (Critical)
- **90/10 Rule (now 95/5):** 95% genuine value, 5% promotion
- **Karma Requirements:** 200+ before promotional posts
- **Account Age:** 30+ days minimum
- **Subreddit Rules:** Each community has additional restrictions
- **Shadowban Risk:** Automated spam detection is aggressive

### The Safe Path
1. **Week 1-2:** Pure karma building (answer questions, be helpful)
2. **Week 3-4:** Add text posts (non-promotional topics)
3. **Week 5:** First "value post" with subtle ArmaraOS mention
4. **Week 6+:** Continued engagement + strategic posts

### What Gets You Banned
❌ Posting about project immediately  
❌ Copy-paste across subreddits  
❌ Multiple accounts upvoting yourself  
❌ Deleting downvoted posts  
❌ Arguing with mods  
❌ Marketing language  

### What Works
✅ Genuine helpfulness  
✅ Story-driven posts  
✅ Humble tone  
✅ Educational value  
✅ Patient karma building  
✅ Community-first mindset  

---

## 🎯 Target Subreddits (Priority Order)

### Tier 1: Primary Targets
1. **r/rust** (500K members) - Technical showcase, "Show-off Saturday"
2. **r/selfhosted** (400K members) - Deployment stories, automation
3. **r/LocalLLaMA** (250K members) - AI agents, local models

### Tier 2: Secondary
4. **r/opensource** (150K members) - Release announcements
5. **r/programming** (5M members) - Architecture, language design
6. **r/artificial** (300K members) - "AI for normal people"

### Tier 3: Niche
7. **r/MachineLearning** (2M members) - Technical ML content
8. **r/devops** (200K members) - Infrastructure automation
9. **r/homelab** (350K members) - Self-hosting enthusiasts

---

## 📅 Content Calendar (First 3 Months)

### Month 1: Foundation
- **Week 1-2:** Karma building only (no posts)
- **Week 3:** First text post (non-promotional)
- **Week 4:** Prepare for launch (200+ karma target)

### Month 2: Launch
- **Week 5:** Post #1 - "Agent Breaking Problem" (r/selfhosted)
- **Week 6:** Post #2 - "Rust Agent OS Showcase" (r/rust)
- **Week 7:** Community engagement only
- **Week 8:** Post #3 - "AI for Normal People" (r/artificial)

### Month 3: Expansion
- **Week 9-10:** Community engagement
- **Week 11:** Post #4 - "AINL Workflow Language" (r/programming)
- **Week 12:** Consider first AMA if karma > 1000

---

## 📝 Ready-to-Use Content

### Post Templates (Copy-Paste Ready)

**Post #1: The Problem Story**
- Subreddit: r/selfhosted
- Format: Text post
- Expected: 100-500 upvotes
- See: `reddit_posts_ready_to_use.md` Line 15

**Post #2: Technical Showcase**
- Subreddit: r/rust
- Format: Text + screenshot
- Expected: 200-800 upvotes
- See: `reddit_posts_ready_to_use.md` Line 79

**Post #3: Accessibility Angle**
- Subreddit: r/artificial
- Format: Discussion post
- Expected: 300-1000 upvotes
- See: `reddit_posts_ready_to_use.md` Line 156

**Post #4: Language Design**
- Subreddit: r/programming
- Format: Code examples
- Expected: 150-600 upvotes
- See: `reddit_posts_ready_to_use.md` Line 236

### Comment Response Templates
All posts include pre-written responses for common questions:
- "What framework is this?"
- "Why Rust?"
- "How do you prevent hallucinations?"
- "What about security?"

---

## 🛠️ Tools & Infrastructure

### Already Available
- **Reddit Adapter:** `~/.openclaw/workspace/armaraos/crates/openfang-channels/src/reddit.rs`
- **OAuth2 Setup:** Built-in Reddit API integration
- **Community Builder:** `~/.openclaw/workspace/agency-agents/marketing/marketing-reddit-community-builder.md`

### External Tools (Free)
- **F5Bot:** Keyword monitoring - https://f5bot.com/
- **Later for Reddit:** Post scheduling - https://laterforreddit.com/
- **Reddit Enhancement Suite:** Browser extension - https://redditenhancementsuite.com/

### Automation (Week 4+)
```bash
# Configure ArmaraOS Reddit adapter
# Edit ~/.armaraos/config.toml:

[channels.reddit]
enabled = true
client_id = "your_reddit_app_id"
client_secret = "your_secret"
username = "armaraos_dev"
password = "your_password"
subreddits = ["rust", "selfhosted", "LocalLLaMA"]
```

---

## 📊 Success Metrics & KPIs

### Week 1 Targets
- [ ] 50+ comment karma
- [ ] Zero rule violations
- [ ] 10+ genuine discussions participated

### Month 1 Targets
- [ ] 200+ total karma
- [ ] 30+ day account age
- [ ] 1-2 posts published
- [ ] Positive community reception

### Month 3 Targets
- [ ] 500+ total karma
- [ ] Recognized username in 2-3 subs
- [ ] 3-4 successful posts (100+ upvotes each)
- [ ] Traffic increase from Reddit referrals

### Month 6 Targets
- [ ] 1000+ total karma
- [ ] Top contributor status
- [ ] AMA completed
- [ ] 15% increase in organic traffic from Reddit
- [ ] Positive brand sentiment (80%+)

---

## ⚠️ Emergency Procedures

### If Account Gets Shadowbanned
1. Visit r/ShadowBan to confirm
2. Contact Reddit admins via reddit.com/appeals
3. Explain situation honestly
4. Wait 60 days before creating new account
5. **DO NOT** create multiple accounts

### If Post Gets Removed
1. Check modmail for reason
2. Apologize politely to mods
3. **DO NOT** argue or repost
4. Wait 2+ weeks before posting to that sub again
5. Learn from feedback

### If Community Backlash
1. **DO NOT** delete posts/comments
2. Respond calmly and honestly
3. Acknowledge mistakes
4. Adjust strategy
5. Come back stronger

---

## 🎓 Learning Resources

### Reddit Official Docs
- Content Policy: https://www.redditinc.com/policies/content-policy
- Self-Promotion: https://www.reddit.com/wiki/selfpromotion
- Moderator Guidelines: https://mods.reddithelp.com/

### Community Guides
- New to Reddit: https://www.reddit.com/r/NewToReddit/wiki/index
- Theory of Reddit: https://www.reddit.com/r/TheoryOfReddit/

### Research Sources
- [Reddit Self-Promotion Rules 2026](https://www.conbersa.ai/learn/reddit-self-promotion-rules)
- [Building Reddit Karma Fast](https://redship.io/blog/build-reddit-karma-fast)
- [Best AI Subreddits 2026](https://aitoolscoutai.com/best-ai-subreddits-2026-guide/)

---

## 🔄 Maintenance Schedule

### Daily (30 min)
- Check comment notifications
- Respond to questions
- Make 3-5 helpful comments
- Upvote relevant content

### Weekly (1 hour)
- Review karma progress
- Participate in "What are you working on?" threads
- Plan next week's engagement
- Check for brand mentions

### Monthly (2 hours)
- Review metrics vs. targets
- Draft next month's posts
- Adjust strategy based on learnings
- Consider AMA if karma > 1000

---

## 📂 File Structure

```
armaraos/
├── REDDIT_CAMPAIGN_README.md          ← You are here
├── REDDIT_MARKETING_STRATEGY.md       ← Complete strategy (110 pages)
├── reddit_posts_ready_to_use.md       ← 4 posts + responses
├── REDDIT_QUICKSTART_GUIDE.md         ← Week 1 setup guide
└── crates/openfang-channels/src/
    └── reddit.rs                      ← Reddit API adapter
```

---

## ✅ Pre-Flight Checklist

Before creating account:
- [ ] Read REDDIT_QUICKSTART_GUIDE.md
- [ ] Review all 4 post templates
- [ ] Understand 95/5 rule
- [ ] Know target subreddit rules
- [ ] Have 30-45 min/day for 4 weeks
- [ ] Committed to patient approach

Before first post (Week 5):
- [ ] 200+ karma achieved
- [ ] 30+ days account age
- [ ] Zero rule violations
- [ ] Post reviewed and ready
- [ ] Optimal time scheduled
- [ ] Prepared to monitor first hour

Before automation (Week 4+):
- [ ] Manual process proven successful
- [ ] Reddit API credentials obtained
- [ ] ArmaraOS adapter configured
- [ ] Monitoring keywords set
- [ ] Response templates ready

---

## 🚀 Execution Status

### Current Phase: Planning Complete ✅
- [x] Research Reddit 2026 rules
- [x] Create comprehensive strategy
- [x] Draft ready-to-use posts
- [x] Prepare quick start guide
- [x] Document automation options

### Next Phase: Account Creation
- [ ] Create account `armaraos_dev`
- [ ] Set up profile (bio, avatar)
- [ ] Subscribe to target subreddits
- [ ] Read all community rules
- [ ] Begin Week 1 karma building

### Future Phases
- [ ] Week 1-4: Karma building (200+ target)
- [ ] Week 5: First post launch
- [ ] Week 6-8: Additional posts
- [ ] Month 3+: Sustainable engagement
- [ ] Month 6: AMA consideration

---

## 💡 Pro Tips

### From the Research
1. **Rising posts > Hot posts** for commenting (better visibility)
2. **Saturday mornings** best for r/rust "Show-off Saturday"
3. **Tuesday-Thursday 9-11 AM EST** best for most subs
4. **Story format > announcement** for engagement
5. **Humble tone > marketing speak** always

### Learned the Hard Way
- Delete nothing (Reddit tracks it)
- Argue with nobody (especially mods)
- Promote sparingly (95/5 rule is real)
- Respond quickly (first hour is critical)
- Learn continuously (adjust based on feedback)

### Success Patterns
- Technical depth + accessible explanation = upvotes
- Personal struggle + solution = engagement
- Controversial opinion + good arguments = discussion
- Humble acknowledgment + transparency = trust

---

## 📞 Support

### Questions About Strategy?
Review: `REDDIT_MARKETING_STRATEGY.md`

### Need Post Templates?
Check: `reddit_posts_ready_to_use.md`

### First Week Confusion?
Read: `REDDIT_QUICKSTART_GUIDE.md`

### Technical Issues?
- Reddit API: `crates/openfang-channels/src/reddit.rs`
- Community Builder: `agency-agents/marketing/marketing-reddit-community-builder.md`

---

## 🎯 Remember

**Reddit is a community, not an advertising platform.**

Success comes from:
- Genuine helpfulness (not strategic promotion)
- Patient relationship building (not quick wins)
- Authentic participation (not marketing campaigns)
- Long-term thinking (months/years, not days/weeks)

You're not "marketing on Reddit" - you're **becoming a valued community member** who happens to build ArmaraOS.

---

## 📝 Change Log

### Version 1.0 (2026-04-22)
- Initial campaign documents created
- Strategy researched and validated
- Posts drafted and reviewed
- Ready for execution

### Next Review: 
After Week 1 execution (2026-04-29)

---

**Document Version:** 1.0  
**Created:** 2026-04-22  
**Status:** Ready to Execute  
**Owner:** Steven Hooley  
**Campaign Goal:** Authentic community building without spam/bans
