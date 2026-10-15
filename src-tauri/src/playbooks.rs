// SPDX-License-Identifier: Apache-2.0
// playbooks.rs — structured negotiation playbooks the agent can follow.
//
// Built-in playbooks cover the situations this assistant is most often asked
// to handle (creditor calls, debt validation, hardship plans, general
// negotiation). Users can add their own by dropping JSON files into
// app-data/playbooks/ — the same drop-in pattern as models. Playbooks are
// deliberately advisory content, not automation: the agent still asks before
// every call, message, or letter, and everything stays approval-gated.

use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    pub id: String,
    pub title: String,
    pub when_to_use: String,
    pub objective: String,
    #[serde(default)]
    pub required_info: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub key_phrases: Vec<String>,
    #[serde(default)]
    pub red_flags: Vec<String>,
    #[serde(default)]
    pub escalation: Vec<String>,
    #[serde(default)]
    pub letter_template: Option<String>,
    /// True for user-supplied files; built-ins are read-only.
    #[serde(default)]
    pub user_supplied: bool,
}

fn built_in_playbooks() -> Vec<Playbook> {
    vec![
        Playbook {
            id: "creditor-negotiation".to_string(),
            title: "Creditor / collections negotiation".to_string(),
            when_to_use: "A past-due balance or collection account needs a settlement or payment arrangement.".to_string(),
            objective: "Reach a settlement or payment plan that the user can actually pay, with the agreement in writing before any money moves."
                .to_string(),
            required_info: vec![
                "Creditor / collector name and the address or number they use".to_string(),
                "Account or reference number".to_string(),
                "Claimed balance and what the user believes is correct".to_string(),
                "Date of last activity and whether the debt is inside the statute of limitations (state-specific)".to_string(),
                "The maximum the user can pay TODAY as a lump sum, and the monthly amount they can sustain".to_string(),
            ],
            steps: vec![
                "Before any call: pull the numbers together and agree the walk-away limits with the user (lump-sum ceiling, monthly ceiling, and 'no deal' line).".to_string(),
                "Open by confirming who you are speaking with and whether they can approve settlements — 'Are you the person who can approve a settlement today, or can you transfer me?'".to_string(),
                "Verify the debt: ask for the original creditor, the date of last activity, and how the balance was calculated. Do NOT confirm or admit the debt is owed.".to_string(),
                "State the hardship honestly and briefly ('a one-time hardship left me with X, and I can pay Y today to resolve this'). Never invent hardship.".to_string(),
                "Make the first offer BELOW the true ceiling (typically 30-40% of the claimed balance for a lump sum on a charged-off account), and let silence do work after the offer.".to_string(),
                "If a number is agreed, stop negotiating and switch to verification: total amount, what happens to the remaining balance, whether the account is reported as 'paid in full', 'settled', or 'paid as agreed', and when the credit bureaus are updated.".to_string(),
                "Get the written agreement (letter or email) BEFORE paying anything, and read it back against what was promised.".to_string(),
                "Pay only by traceable means (money order, cashier's check, or the creditor's own portal). Never give bank account credentials or a debit card for a 'one-time draft' to a collector.".to_string(),
                "After payment: keep the confirmation, the written agreement, and a record of the payment; follow up in 30 days to confirm the balance is zero and the bureau reporting is correct.".to_string(),
            ],
            key_phrases: vec![
                "I'm not confirming the debt — I'm trying to resolve this in good faith.".to_string(),
                "What's your client's settlement authority on this account today?".to_string(),
                "If we agree on a number, will you send me that in writing before I pay?".to_string(),
                "I can do X today as a full and final settlement. If that works, how do we document it?".to_string(),
                "I'll hold while you check with a supervisor.".to_string(),
            ],
            red_flags: vec![
                "Pressure to pay immediately without anything in writing".to_string(),
                "Requests for bank account credentials, full debit-card numbers, or gift cards".to_string(),
                "Refusal to confirm the original creditor or the date of last activity".to_string(),
                "Threats of immediate arrest, wage garnishment without a court order, or claims they cannot prove".to_string(),
                "Any promise that is 'just how it goes' but that they will not put in writing".to_string(),
            ],
            escalation: vec![
                "Ask for a supervisor when the representative cannot approve a settlement.".to_string(),
                "If the collector misrepresents the debt, threatens illegal action, or calls after a written cease request, note the date, time, name, and what was said — that is potential FDCPA material for a complaint."
                    .to_string(),
                "Escalate to the original creditor's hardship or executive-resolution team if the debt was sold but the original creditor matters to the user.".to_string(),
                "File complaints with the CFPB (consumerfinance.gov) and the state attorney general when a collector violates the rules; keep every reference number.".to_string(),
            ],
            letter_template: Some(
                "Re: Account [ACCOUNT/REF]\n\
Creditor/Collector: [NAME]\n\
Date: [DATE]\n\n\
To whom it may concern:\n\n\
I am writing regarding the above account. This letter is not an acknowledgment that I owe the debt; it is a good-faith attempt to resolve this matter.\n\n\
I dispute the amount claimed. Please provide validation of the debt, including the name of the original creditor, the date of last activity, and an accounting of how the balance, interest, and fees were calculated.\n\n\
If you can agree to a full and final settlement of [AMOUNT] paid by [DATE] / by [METHOD], and if you will confirm in writing that the remaining balance will not be sold or pursued and how the account will be reported to the credit bureaus, I am prepared to resolve this voluntarily.\n\n\
Please direct all communication to me in writing at the address above.\n\n\
Sincerely,\n[JESSE MACHUCA]"
                    .to_string(),
            ),
            user_supplied: false,
        },
        Playbook {
            id: "debt-validation".to_string(),
            title: "Debt validation / dispute".to_string(),
            when_to_use: "A collector appears with a debt the user does not recognize, or the amount looks wrong.".to_string(),
            objective: "Force the collector to prove the debt before any payment discussion, and preserve the user's dispute rights."
                .to_string(),
            required_info: vec![
                "Collector name, address, phone, and the account/reference number they used".to_string(),
                "Date of their first contact (the 30-day validation window starts there)".to_string(),
                "What the user actually remembers about the original account".to_string(),
            ],
            steps: vec![
                "Do not confirm or deny the debt in the first contact. Say only: 'I am not confirming this debt and I am requesting validation in writing.'".to_string(),
                "Send the validation letter by certified mail (return receipt) within 30 days of first contact — keep the receipt and a copy.".to_string(),
                "Log what arrives: a proper validation includes the original creditor, the amount, and how it was computed. A form letter with no accounting is not validation.".to_string(),
                "If validation never arrives, the collector must stop collection until it does — document every contact after the request.".to_string(),
                "Dispute with the credit bureaus separately (Equifax, Experian, TransUnion) with the same facts and copies of the letters.".to_string(),
                "Only after validation arrives (or the debt is proven) decide between settlement, payment plan, or continued dispute.".to_string(),
            ],
            key_phrases: vec![
                "I am not confirming this debt. I am requesting validation.".to_string(),
                "Please send the validation in writing to the address of record.".to_string(),
                "What is the date of first delinquency you are reporting, and to which bureaus?".to_string(),
            ],
            red_flags: vec![
                "Collector refuses to give a mailing address".to_string(),
                "Validation never arrives but collection attempts continue".to_string(),
                "The account first appears on the credit report without prior contact".to_string(),
            ],
            escalation: vec![
                "If collection continues without validation, that is a potential FDCPA violation — file with the CFPB and the state attorney general, and notify the collector in writing that the contact is disputed."
                    .to_string(),
                "For identity-theft-shaped debts, add a police report and FTC identity-theft report to the dispute packet.".to_string(),
            ],
            letter_template: Some(
                "Re: Account [ACCOUNT/REF]\n\
Collector: [NAME]\n\
Date: [DATE]\n\n\
To whom it may concern:\n\n\
I am not confirming that I owe this debt. This is a request for validation under the Fair Debt Collection Practices Act.\n\n\
Please provide: (1) the name and address of the original creditor; (2) the amount you claim is owed and an itemization of the principal, interest, and fees; (3) the date of first delinquency and the date of last activity; (4) documentation that your company owns the debt or is authorized to collect it.\n\n\
Until validation is provided, please cease all collection activity. Communication should be in writing only at the address above.\n\n\
Sincerely,\n[JESSE MACHUCA]"
                    .to_string(),
            ),
            user_supplied: false,
        },
        Playbook {
            id: "hardship-payment-plan".to_string(),
            title: "Hardship payment plan request".to_string(),
            when_to_use: "The user can pay something monthly but not the full demanded amount, and wants to keep the account out of collections.".to_string(),
            objective: "Convert an unaffordable demand into a written, sustainable payment plan with reduced or frozen interest."
                .to_string(),
            required_info: vec![
                "Monthly amount the user can commit to without failing".to_string(),
                "The number of months they can sustain it".to_string(),
                "Whether interest can be frozen and fees waived as part of the plan".to_string(),
            ],
            steps: vec![
                "Ask for the hardship or loss-mitigation department rather than the general collections line.".to_string(),
                "Present the numbers plainly: what the user can pay, for how long, starting when.".to_string(),
                "Ask for interest freeze, fee waivers, and — importantly — that payments count as on-time and the account stays current.".to_string(),
                "Get the plan in writing (amount, dates, consequences of a missed payment, and what happens at the end).".to_string(),
                "Set up automatic payments only through the creditor's own portal, and only for the agreed amount.".to_string(),
                "Check back in 30 days that payments are posting correctly and that the account shows as current.".to_string(),
            ],
            key_phrases: vec![
                "I can commit to [AMOUNT] on the [DATE] of each month for [N] months if we can freeze the interest and stop the fees.".to_string(),
                "Can we put that arrangement in writing before the first payment?".to_string(),
                "If a payment is ever late, what happens to the plan? I want to know the terms up front.".to_string(),
            ],
            red_flags: vec![
                "'Verbal only' promises with no written terms".to_string(),
                "Requests for a large 'good-faith' first payment before any written plan".to_string(),
                "Terms that reset the clock or waive the user's dispute rights".to_string(),
            ],
            escalation: vec![
                "If the front line cannot approve it, request a supervisor or the hardship department in writing.".to_string(),
                "If the account is reported late while a written plan is being honored, dispute the bureau report with the plan letter attached.".to_string(),
            ],
            letter_template: Some(
                "Re: Account [ACCOUNT/REF]\n\
Date: [DATE]\n\n\
To whom it may concern:\n\n\
I am requesting a hardship payment arrangement on the above account.\n\n\
My circumstances: [BRIEF, HONEST STATEMENT].\n\n\
I can commit to [AMOUNT] on the [DATE] of each month for [N] months, starting [FIRST PAYMENT DATE]. In exchange, I am asking for: (1) interest to be frozen; (2) late/over-limit fees to be waived; (3) payments to be reported as agreed; and (4) the arrangement confirmed in writing before the first payment.\n\n\
Please send the written terms to the address above.\n\n\
Sincerely,\n[JESSE MACHUCA]"
                    .to_string(),
            ),
            user_supplied: false,
        },
        Playbook {
            id: "generic-negotiation".to_string(),
            title: "General negotiation framework".to_string(),
            when_to_use: "Any negotiation without a domain-specific playbook (bills, refunds, vendors, fees).".to_string(),
            objective: "Reach the best outcome the other side will actually deliver, without burning the relationship."
                .to_string(),
            required_info: vec![
                "What the user wants, concretely, in one sentence".to_string(),
                "The best alternative if there is no agreement (the walk-away)".to_string(),
                "The lowest/highest number the user will accept, and the opening number".to_string(),
                "What the other side plausibly needs to say yes (policy cover, timing, volume, a clean file)".to_string(),
            ],
            steps: vec![
                "Open by stating the goal and asking what is possible, before naming numbers wherever possible.".to_string(),
                "Anchor first with a defensible number and justify it with a specific fact, not emotion.".to_string(),
                "Trade, don't just take: every concession you grant is named as conditional ('if you can do X, I can do Y today').".to_string(),
                "Use silence after an offer; most people fill it, often with a better number.".to_string(),
                "Summarize the deal back in the other side's words, then get it in writing.".to_string(),
                "If the other side cannot decide, find the person who can, politely and explicitly.".to_string(),
            ],
            key_phrases: vec![
                "Help me understand what is possible here.".to_string(),
                "If we could do X, would that work on your side?".to_string(),
                "Before we finalize — who else needs to approve this?".to_string(),
                "Let's write down what we just agreed to so we both have it.".to_string(),
            ],
            red_flags: vec![
                "Verbal assurances with nothing in writing".to_string(),
                "Pressure to decide immediately".to_string(),
                "New terms appearing after agreement".to_string(),
            ],
            escalation: vec![
                "Ask for a supervisor once, calmly, with the facts in one sentence.".to_string(),
                "If the counterparty is a company, written complaints to a regulator often move more than further phone calls.".to_string(),
            ],
            letter_template: None,
            user_supplied: false,
        },
    ]
}

/// User playbooks: JSON files dropped into app-data/playbooks/.
fn user_playbooks(app: &tauri::AppHandle) -> Vec<Playbook> {
    let dir = match app.path().app_data_dir() {
        Ok(d) => d.join("playbooks"),
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e.eq_ignore_ascii_case("json")) != Some(true) {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(mut pb) = serde_json::from_str::<Playbook>(&raw) {
                pb.user_supplied = true;
                out.push(pb);
            }
        }
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

/// All playbooks: built-ins first, then user-supplied (which may override by id).
pub fn registry(app: &tauri::AppHandle) -> Vec<Playbook> {
    let mut all = built_in_playbooks();
    let user = user_playbooks(app);
    all.retain(|b| !user.iter().any(|u| u.id == b.id));
    all.extend(user);
    all
}

pub fn get(app: &tauri::AppHandle, id: &str) -> Option<Playbook> {
    registry(app).into_iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_playbooks_are_complete_and_unique() {
        let books = built_in_playbooks();
        assert!(books.len() >= 4, "expected the starter library");
        let mut ids: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), books.len(), "playbook ids must be unique");
        for b in &books {
            assert!(!b.title.trim().is_empty(), "{}: title", b.id);
            assert!(!b.objective.trim().is_empty(), "{}: objective", b.id);
            assert!(b.steps.len() >= 3, "{} must have real steps", b.id);
            assert!(b.red_flags.len() >= 1, "{} must warn about red flags", b.id);
        }
    }

    #[test]
    fn creditor_playbook_teaches_written_agreement_and_no_credentials() {
        let books = built_in_playbooks();
        let cred = books
            .iter()
            .find(|b| b.id == "creditor-negotiation")
            .expect("creditor playbook");
        let joined = format!(
            "{} {} {}",
            cred.steps.join(" "),
            cred.key_phrases.join(" "),
            cred.red_flags.join(" ")
        )
        .to_lowercase();
        assert!(joined.contains("in writing"), "must demand written agreements");
        assert!(
            joined.contains("bank account credentials"),
            "must forbid handing over banking credentials"
        );
        assert!(
            cred.letter_template.as_deref().unwrap_or("").contains("not an acknowledgment"),
            "letter must not admit liability"
        );
    }
}
