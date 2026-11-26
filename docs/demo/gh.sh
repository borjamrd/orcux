#!/bin/sh
# A pretend gh for the demo: it answers only orcux's read of open pull requests, with made-up ones, as gh would after
# its --jq (one TSV line per PR). Anything else fails: nothing reaches GitHub.
# kind repo number title url draft decision mergeable branch fork author ci age ok failed total
case "$*" in
"auth status"*) printf 'demo\tGH_TOKEN\n' ;;
"auth token"*) echo demo-token ;;
"api graphql"*"m=is:pr"*)
    sed 's/|/	/g' <<'EOF'
mine|demo/shop|212|Fix the redirect after sign-in|https://github.com/demo/shop/pull/212|false|CHANGES_REQUESTED|MERGEABLE|login-fix|false|demo|SUCCESS|3600|5|0|5
review|demo/shop|208|Product reviews|https://github.com/demo/shop/pull/208|false|REVIEW_REQUIRED|MERGEABLE|reviews|false|alex|SUCCESS|7200|5|0|5
mine|demo/shop|214|Search by product name|https://github.com/demo/shop/pull/214|true|REVIEW_REQUIRED|MERGEABLE|search|false|demo|SUCCESS|900|5|0|5
mine|demo/api|87|Limit requests per customer|https://github.com/demo/api/pull/87|false|REVIEW_REQUIRED|MERGEABLE|rate-limit|false|demo|PENDING|300|3|0|5
mine|demo/api|84|Paginate the orders|https://github.com/demo/api/pull/84|false|APPROVED|MERGEABLE|orders-page|false|demo|SUCCESS|86400|6|0|6
EOF
    ;;
*) exit 1 ;;
esac
