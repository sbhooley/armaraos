# Reddit Account Creation & First Week Guide
## Get Started in 15 Minutes

**Last Updated:** 2026-04-22

---

## Step 1: Create Reddit Account (5 minutes)

### Go to Reddit
1. Visit https://www.reddit.com/ in private/incognito browser
2. Click "Sign Up" in top right

### Account Details
- **Username:** `armaraos_dev` (or similar - check availability)
  - Alternatives if taken: `armaraos_builder`, `armaraos_hacker`, `building_armaraos`
  - **Avoid:** `ArmaraOS_Official`, `ArmaraOS_Team` (looks corporate)

- **Password:** Use strong password, save in password manager

- **Email:** Use real email (for account recovery)
  - **Tip:** Create a dedicated email like `armaraos.reddit@gmail.com`

### Complete Signup
1. Verify email (check inbox)
2. **DO NOT** skip through onboarding - select relevant interests
3. Choose interests:
   - ✅ Programming
   - ✅ Technology
   - ✅ Open Source
   - ✅ Rust
   - ✅ Self Hosting

### Set Up Profile (3 minutes)

1. Click your username → "Profile"

2. **Bio:**
   ```
   Building autonomous agent systems in Rust 🦀 | Open source enthusiast | Coffee-powered developer
   ```

3. **Avatar:**
   - Option 1: Reddit's avatar builder (pick something tech/robot themed)
   - Option 2: Simple Rust logo (don't use ArmaraOS branding yet)

4. **Banner:** Leave blank or use generic developer theme

5. **Links:** Leave blank for now (can add GitHub after Week 3)

---

## Step 2: Subscribe to Target Subreddits (5 minutes)

### Primary Subreddits (Subscribe Now)
1. r/rust - https://reddit.com/r/rust
2. r/selfhosted - https://reddit.com/r/selfhosted
3. r/programming - https://reddit.com/r/programming
4. r/opensource - https://reddit.com/r/opensource
5. r/LocalLLaMA - https://reddit.com/r/LocalLLaMA

### Secondary (Optional, Subscribe Later)
- r/artificial
- r/MachineLearning
- r/devops
- r/homelab

### How to Subscribe
1. Visit subreddit (click links above)
2. Click "Join" button on right sidebar
3. Turn on notifications (optional - but useful for trending posts)

---

## Step 3: Read the Rules (15 minutes - CRITICAL)

### For Each Primary Subreddit:

1. Click subreddit name
2. Look for "Rules" in sidebar (or "About" on mobile)
3. Read ALL rules carefully
4. Note these specifically:
   - Self-promotion policies
   - Link posting requirements
   - Minimum karma requirements
   - Banned content types

### Key Rules to Remember:

**r/rust:**
- No low-effort posts
- "Show-off Saturday" for projects
- Technical discussions welcome any day
- No spam/self-promotion outside Saturday

**r/selfhosted:**
- Must be self-hostable (no cloud-only)
- Provide value to community
- Technical details required
- No "check out my SaaS" posts

**r/programming:**
- VERY strict moderation
- No "Show HN" style posts
- Must be about programming concepts
- High quality bar

**r/LocalLLaMA:**
- Focus on local/open models
- No pure cloud solutions
- Technical discussion encouraged
- Help posts welcome

---

## Week 1: Karma Building (NO PROMOTION)

**Goal:** 50-100 comment karma  
**Time:** 30-45 minutes per day  
**Strategy:** Be genuinely helpful

### Daily Routine (30 minutes)

#### Morning (15 minutes)
1. **Check r/rust "New" tab**
   - Sort by "New"
   - Find beginner questions
   - Answer 1-2 questions with genuine help

   Example good comment:
   ```
   I ran into this exact issue last week! The problem is [explain]. 
   
   Here's what worked for me:
   ```rust
   // helpful code example
   ```
   
   The key is [explanation of why it works].
   ```

2. **Check r/selfhosted "Rising"**
   - Sort by "Rising"
   - Comment on posts gaining traction
   - Share relevant experience

   Example:
   ```
   This is a great use case for [related technology]. 
   I've been running something similar for [timeframe] and the biggest challenge is [specific issue].
   
   If you haven't already, check out [genuinely helpful resource].
   ```

#### Evening (15 minutes)
1. **Respond to your comments**
   - Check notifications
   - Reply to follow-up questions
   - Thank people for upvotes (optional)

2. **Find "What are you working on?" threads**
   - r/rust has weekly thread (usually Friday)
   - Share BRIEF update (2-3 sentences)
   
   Example:
   ```
   Working on agent orchestration in Rust. 
   This week: Implementing graph-native memory. 
   The borrow checker is teaching me patience 😅
   ```

### What to Comment On:

✅ **Good Topics:**
- Beginner Rust questions
- Architecture discussions
- Tool recommendations (third-party tools)
- Deployment challenges
- "What's everyone working on?" threads

❌ **Avoid:**
- Politics or controversial topics
- Arguments or flame wars
- Anything promotional
- Off-topic discussions

### Example High-Quality Comments:

**Type 1: Helpful Answer**
```
For async Rust, the key insight that helped me was understanding Send + Sync bounds.

TL;DR:
- Send = safe to move between threads
- Sync = safe to reference from multiple threads

Most Tokio types are Send but not Sync. So Arc<Mutex<T>> becomes your friend.

Jon Gjengset's video on this is gold: [YouTube link]
```

**Type 2: Sharing Experience**
```
I tried the same approach last month and hit a wall with [specific issue].

Ended up switching to [alternative approach] which solved it but introduced [new challenge].

Trade-off was worth it for my use case (24/7 daemon), but YMMV depending on your requirements.
```

**Type 3: Adding Value**
```
Great point about the memory overhead.

For anyone curious, I benchmarked this:
- Approach A: 40MB baseline, 2ms latency
- Approach B: 120MB baseline, 0.5ms latency

Depends on whether you're optimizing for memory or speed.
```

---

## Week 1 Checklist

### Monday
- [x] Created Reddit account
- [x] Set up profile
- [x] Subscribed to 5 primary subreddits
- [ ] Read all subreddit rules
- [ ] Made 3 helpful comments
- [ ] Zero mentions of ArmaraOS/AINL

### Tuesday-Friday (Each Day)
- [ ] Spent 30 minutes on Reddit
- [ ] Made 3-5 helpful comments
- [ ] Responded to any replies
- [ ] Upvoted helpful posts
- [ ] Zero promotional activity

### Saturday
- [ ] Participated in r/rust "What are you working on?" thread
- [ ] Reviewed karma progress (target: 50+)
- [ ] Adjusted strategy if needed

### Sunday
- [ ] Light engagement (1-2 comments)
- [ ] Reviewed week's activity
- [ ] Planned next week

---

## Karma Tracking

### Daily Log (Keep in Notes)
```
Date: 2026-04-22
Comments: 4
Upvotes received: ~12
Total karma: 47
Best comment: "Async Rust explanation" (+8)
Lessons: Technical help gets more upvotes than opinions
```

### Milestone Targets
- [ ] Day 3: 20 karma
- [ ] Week 1: 50 karma
- [ ] Week 2: 100 karma
- [ ] Week 3: 150 karma
- [ ] Week 4: 200 karma (ready for first post)

---

## Common Mistakes to Avoid

### Week 1 Killers:
❌ Posting about your project immediately
❌ Including GitHub links in comments
❌ Generic comments like "Nice!" or "Great post!"
❌ Commenting more than 5 times per day (looks like spam)
❌ Arguing with people (downvote magnet)
❌ Deleting comments that get downvoted (Reddit tracks this)

### Subtle Red Flags:
❌ Always steering conversations to "agents" or "Rust"
❌ Mentioning your work in every thread
❌ Upvoting only your own comments
❌ Using AI-generated comments (people can tell)
❌ Commenting at weird hours (3 AM looks like bot)

---

## When Things Go Wrong

### If You Get Downvoted:
1. **Don't panic** - 1-2 downvotes are normal
2. **Don't delete** - Looks suspicious
3. **Learn** - Was comment off-topic? Too promotional?
4. **Adjust** - Next comment should be more helpful

### If Comment Gets No Response:
1. **Normal** - Most comments get 0-3 upvotes
2. **Timing** - Try commenting on "Rising" posts, not "Hot"
3. **Quality** - Add more value (examples, code, links)

### If You're Tempted to Promote:
1. **Wait** - You're not ready yet
2. **Resist** - One premature promotion kills weeks of work
3. **Refocus** - Go help someone instead

---

## Week 2-4 Preview

### Week 2: Continue Building
- Keep commenting (3-5/day)
- Start upvoting others' posts
- Engage in longer discussions
- **Still no promotion**

### Week 3: Add Text Posts
- Submit 1-2 non-promotional text posts
- Examples: "TIL about [tech]", "Question about [topic]"
- **Still no links to your projects**

### Week 4: Prepare for Launch
- Hit 200+ karma target
- Draft first value post
- Get feedback from trusted source
- Plan posting time carefully

---

## Tools & Resources

### Reddit Enhancement Suite (Browser Extension)
- Better comment navigation
- Karma breakdown by subreddit
- User tagging
- Download: https://redditenhancementsuite.com/

### Mobile Apps (Better than Reddit app)
- **Apollo** (iOS) - Best Reddit app
- **Relay** (Android) - Great for commenting
- **Old Reddit** (web) - https://old.reddit.com (cleaner interface)

### Monitoring Tools (Use After Week 4)
- **F5Bot** - Keyword alerts: https://f5bot.com/
- **Later for Reddit** - Optimal posting times: https://laterforreddit.com/

---

## Quick Reference Card

```
┌─────────────────────────────────────────────────┐
│         REDDIT WEEK 1 QUICK REFERENCE           │
├─────────────────────────────────────────────────┤
│ Username: armaraos_dev                          │
│ Target Karma: 50 by end of week                 │
│ Daily Time: 30-45 minutes                       │
│                                                  │
│ DO:                                              │
│ ✓ Answer questions                              │
│ ✓ Share experiences                             │
│ ✓ Be helpful                                    │
│ ✓ Respond to replies                            │
│ ✓ Upvote good content                           │
│                                                  │
│ DON'T:                                           │
│ ✗ Mention ArmaraOS                              │
│ ✗ Post links                                    │
│ ✗ Argue                                         │
│ ✗ Delete comments                               │
│ ✗ Comment > 5 times/day                         │
│                                                  │
│ Target Subreddits:                               │
│ • r/rust (answer questions)                     │
│ • r/selfhosted (share experience)               │
│ • r/programming (thoughtful comments)           │
│                                                  │
│ Next Milestone: Week 4 (200 karma, first post)  │
└─────────────────────────────────────────────────┘
```

---

## Daily Checklist (Print This)

```
☐ Check r/rust "New" (find questions to answer)
☐ Check r/selfhosted "Rising" (comment on trending)
☐ Respond to my comments (engage with replies)
☐ Upvote 5-10 helpful posts
☐ Made 3-5 quality comments
☐ Zero promotional activity
☐ Learned something new
☐ End karma: _____
```

---

## Success Stories (What Good Looks Like)

### Example Day 1:
- Comments: 3
- Karma gained: +8
- Best comment: Helped someone with Tokio issue (+5)
- Time spent: 35 minutes

### Example Week 1:
- Total comments: 22
- Karma gained: +47
- Best thread: Participated in "What are you working on?" (+12)
- No violations, no downvotes
- Ready for Week 2

---

## Emergency Contact

If you're stuck or unsure:
1. Check the strategy doc: `REDDIT_MARKETING_STRATEGY.md`
2. Review post templates: `reddit_posts_ready_to_use.md`
3. Re-read subreddit rules
4. When in doubt: **Don't post, just comment**

---

## Final Tips

🎯 **Success = Patience + Authenticity**

The accounts that succeed:
- Build slowly (weeks, not days)
- Help genuinely (not strategically)
- Participate consistently (daily habits)
- Stay humble (acknowledge mistakes)

The accounts that fail:
- Rush to promote (ban within days)
- Spam links (shadowban)
- Argue defensively (downvote spiral)
- Give up early (karma takes time)

You're building a **reputation**, not running a **campaign**.

Good luck! 🚀

---

**Document Version:** 1.0  
**Next Review:** After Week 1  
**Questions?** Review the full strategy doc
