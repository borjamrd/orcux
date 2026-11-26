#!/usr/bin/perl
# A pretend coding agent for the demo: it draws what agents draw (a spinner over "esc to cancel", a permission
# menu, a finished answer) and tells orcux its state with "orcux hook <state> exact". No real agent runs.
#   perl agent.pl work|ask|done|lead|sub [task]
# lead waits for a task, then hands three parts of it to sub-agents in child worktrees (orcux run --parent) and waits
# for them; sub is one of those.
use strict;
use warnings;
use utf8;
use Time::HiRes qw(sleep time);
binmode STDOUT, ':utf8';
$| = 1;

my $role = shift // 'work';
my $given = shift // '';
my ($rows, $cols) = (24, 80);
my $resized = 1;
$SIG{WINCH} = sub { $resized = 1 };
my @spin = split //, '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏';
my ($dim, $bold, $blue, $green, $yellow, $off) = ("\e[90m", "\e[1m", "\e[38;5;110m", "\e[38;5;114m", "\e[38;5;179m", "\e[0m");
my $input = '';
my $start = time - 37;

sub size { my $s = qx(stty size 2>/dev/null); ($rows, $cols) = split ' ', $s if $s =~ /^\d+ \d+/ }
sub hook { system('orcux', 'hook', $_[0], 'exact') }

# the body at the top, the input box and the footer at the bottom
sub frame {
    my ($body, $foot) = @_;
    size() if $resized;
    $resized = 0;
    my $w = $cols - 4;
    my @in;
    for my $l (split /\r?\n|\r/, ($input eq '' ? ' ' : $input)) {
        my $line = '';
        for my $word (split / /, $l) {
            if ($line ne '' && length($line) + 1 + length($word) > $w) { push @in, $line; $line = $word }
            else { $line = $line eq '' ? $word : "$line $word" }
        }
        push @in, $line;
    }
    # every row written over in place, never cleared first: a screen read meanwhile is never half empty
    my @screen = split /\n/, $body;
    my $at = $rows - @in - 1;
    $screen[$at - 2] = "$dim" . ('─' x $cols) . $off;
    $screen[$at - 1 + $_] = ($_ ? '  ' : "$bold>$off ") . $in[$_] for 0 .. $#in;
    $screen[$at + @in - 1] = "$dim" . ('─' x $cols) . $off;
    $screen[$rows - 1] = "$dim$foot$off";
    my $out = "\e[?25l";
    $out .= "\e[" . ($_ + 1) . ";1H" . ($screen[$_] // '') . "\e[0m\e[K" for 0 .. $rows - 1;
    print $out;
}

my $task = {
    work => $given || 'Add search by product name to the catalog',
    ask  => 'Fix the redirect after sign-in',
    done => 'Show the cart total on the checkout page',
    lead => '',
    sub  => $given,
}->{$role};
my $head = "$bold> $task$off\n\n";

sub step { my ($verb, $what, $rest) = @_; "$blue•$off $verb ${bold}$what$off" . ($rest // '') . "\n" }
sub diff { "  $green+$_[0]$off" . (defined $_[1] ? " $yellow-$_[1]$off" : '') }
sub result { "  ${dim}└ $_[0]$off\n" }

my %steps = (
    work => $given =~ /request/i
        ? step('Read', 'src/limits.ts') . step('Edit', 'src/limits.ts', diff(12, 1)) . step('Run', 'npm test -- limits') . result('7 passed')
        : step('Read', 'src/catalog/search.ts') . step('Edit', 'src/catalog/search.ts', diff(24, 3)) . step('Run', 'npm test -- search') . result('18 passed'),
    ask  => step('Read', 'src/auth/redirect.ts') . step('Edit', 'src/auth/redirect.ts', diff(6, 2)),
    done => step('Read', 'src/cart/total.ts') . step('Edit', 'src/checkout/Summary.tsx', diff(4, 1)) . step('Edit', 'src/cart/total.ts', diff(5, 2)) . step('Run', 'npm test -- cart') . result('23 passed'),
);

sub spinner { my ($i, $what, $secs) = @_; "\n$blue$spin[$i % @spin]$off  $what ${dim}" . int($secs) . "s$off" }

sub working {
    my ($body) = @_;
    hook('working');
    my $i = 0;
    while (1) {
        frame($head . $body . spinner($i++, 'Working…', time - $start), 'esc to cancel');
        sleep 0.12;
    }
}

# keys until there are some (0.2 s at most), redrawing on a resize; undef when the terminal closed
sub keys_in {
    my ($draw) = @_;
    my $rin = '';
    vec($rin, fileno(STDIN), 1) = 1;
    if (select(my $r = $rin, undef, undef, 0.2) > 0) {
        sysread(STDIN, my $c, 4096) or return undef;
        utf8::decode($c);
        return $c;
    }
    $draw->() if $resized;
    return '';
}

# keys typed or pasted go to the input box; nothing is ever sent
sub typing {
    my ($draw) = @_;
    system('stty', 'raw', '-echo');
    $draw->();
    while (defined(my $c = keys_in($draw))) {
        next if $c eq '';
        for my $ch (split //, $c) {
            if ($ch eq "\x7f") { chop $input }
            elsif ($ch eq "\r" || $ch eq "\n") { $input .= "\n" }
            elsif ($ch eq "\x03") { system('stty', 'sane'); exit 0 }
            elsif (ord $ch >= 32) { $input .= $ch }
        }
        $draw->();
    }
}

if ($role eq 'work') {
    working($steps{work});
} elsif ($role eq 'ask') {
    hook('permission');
    my $menu = "\n${yellow}Requesting permission for:$off\n   ${bold}npm test -- auth$off\nRun this command?\n$blue>$off 1. Yes, run command\n  2. No, cancel\n";
    my $draw = sub { frame($head . $steps{ask} . $menu, 'esc to cancel') };
    system('stty', 'raw', '-echo');
    $draw->();
    while (defined(my $c = keys_in($draw))) { last if $c =~ /[1\r]/ }
    system('stty', 'sane');
    $start = time;
    working($steps{ask} . step('Run', 'npm test -- auth') . result('12 passed') . step('Edit', 'src/auth/session.ts', diff(3)));
} elsif ($role eq 'sub') {
    # the parts end one after the other
    my $secs = $task =~ /cart/ ? 3 : $task =~ /address/ ? 4.5 : 6;
    hook('working');
    my ($i, $t0) = (0, time);
    my $body = step('Read', 'src/checkout/Summary.tsx');
    while (time - $t0 < $secs) {
        frame($head . $body . spinner($i++, 'Working…', time - $t0), 'esc to cancel');
        sleep 0.12;
    }
    hook('done');
    $body .= step('Edit', 'src/checkout/steps.tsx', diff(31)) . step('Run', 'npm test -- checkout') . result('9 passed');
    typing(sub { frame($head . $body . "\nDone: the step is in, with its test.\n", '? for shortcuts') });
} elsif ($role eq 'lead') {
    # waits for the person's task, typed in its input
    hook('idle');
    my $draw = sub { frame("${dim}Ready for a task.$off\n", '? for shortcuts') };
    system('stty', 'raw', '-echo');
    $draw->();
    my $sent = 0;
    while (!$sent && defined(my $c = keys_in($draw))) {
        next if $c eq '';
        for my $ch (split //, $c) {
            if ($ch eq "\r") { $sent = $input ne ''; last }
            if ($ch eq "\x7f") { chop $input } elsif (ord $ch >= 32) { $input .= $ch }
        }
        $draw->();
    }
    $head = "$bold> $input$off\n\n";
    $input = '';
    $start = time;
    hook('working');
    my $body = step('Read', 'src/checkout/Summary.tsx') . result('three steps that touch different files');
    my $me = $ENV{ORCUX_ID} // 'shop/checkout';
    for my $p ('cart step', 'address step', 'payment step') {
        (my $name = $p) =~ s/ /-/g;
        $body .= step('Run', qq{orcux run "$p" --parent $me});
        frame($head . $body, 'esc to cancel');
        open(my $run, '-|', 'orcux', 'run', ucfirst("$p of the checkout page"), '--parent', $me, '--agent', 'sub', '--name', $name) or die;
        my ($port) = (do { local $/; <$run> } // '') =~ /ORCUX_PORT=(\d+)/;
        close $run;
        $body .= result("shop/$name  ORCUX_PORT=" . ($port // '?'));
        frame($head . $body, 'esc to cancel');
        sleep 0.4;
    }
    $body .= step('Run', 'orcux wait cart-step address-step payment-step');
    my $pid = fork();
    if (!$pid) {
        open(STDOUT, '>', '/dev/null');
        exec('orcux', 'wait', 'cart-step', 'address-step', 'payment-step', '--timeout', '120');
        exit 1;
    }
    my $i = 0;
    while (waitpid($pid, 1) == 0) {
        frame($head . $body . spinner($i++, 'Waiting for 3 agents…', time - $start), 'esc to cancel');
        sleep 0.12;
    }
    $body .= result('3 done');
    hook('done');
    typing(sub { frame($head . $body . "\nThe checkout has its three steps, each in a branch of\nits own, tested. Merge each into this one with ${bold}m$off.\n", '? for shortcuts') });
} else {
    hook('done');
    my $answer = "\nThe checkout page shows the cart total now, shipping\nincluded. Committed as ${bold}Show the cart total at checkout$off;\nthe rounding in ${bold}src/cart/total.ts$off waits for your review.\n";
    typing(sub { frame($head . $steps{done} . $answer, '? for shortcuts') });
}
