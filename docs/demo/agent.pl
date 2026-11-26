#!/usr/bin/perl
# A pretend coding agent for the demo: it draws what agents draw (a spinner over "esc to cancel", a permission
# menu, a finished answer) and tells orcux its state with "orcux hook <state> exact". No real agent runs.
# perl agent.pl work|ask|done
use strict;
use warnings;
use utf8;
use Time::HiRes qw(sleep time);
binmode STDOUT, ':utf8';
$| = 1;

my $role = shift // 'work';
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
    my @body = split /\n/, $body;
    my $out = "\e[?25l\e[H\e[2J";
    $out .= "$_\r\n" for @body;
    my $at = $rows - @in - 1;
    $out .= "\e[" . ($at - 1) . ";1H$dim" . ('─' x $cols) . $off;
    for my $i (0 .. $#in) { $out .= "\e[" . ($at + $i) . ";1H" . ($i ? '  ' : "$bold>$off ") . $in[$i] }
    $out .= "\e[" . ($at + @in) . ";1H$dim" . ('─' x $cols) . $off;
    $out .= "\e[$rows;1H$dim$foot$off";
    print $out;
}

my $task = {
    work => 'Add search by product name to the catalog',
    ask  => 'Fix the redirect after sign-in',
    done => 'Show the cart total on the checkout page',
}->{$role};
my $head = "$bold> $task$off\n\n";

sub step { my ($verb, $what, $rest) = @_; "$blue•$off $verb ${bold}$what$off" . ($rest // '') . "\n" }
sub diff { "  $green+$_[0]$off" . (defined $_[1] ? " $yellow-$_[1]$off" : '') }
sub result { "  ${dim}└ $_[0]$off\n" }

my %steps = (
    work => step('Read', 'src/catalog/search.ts') . step('Edit', 'src/catalog/search.ts', diff(24, 3)) . step('Run', 'npm test -- search') . result('18 passed'),
    ask  => step('Read', 'src/auth/redirect.ts') . step('Edit', 'src/auth/redirect.ts', diff(6, 2)),
    done => step('Read', 'src/cart/total.ts') . step('Edit', 'src/checkout/Summary.tsx', diff(4, 1)) . step('Edit', 'src/cart/total.ts', diff(5, 2)) . step('Run', 'npm test -- cart') . result('23 passed'),
);

sub working {
    my ($body) = @_;
    hook('working');
    my $i = 0;
    while (1) {
        my $secs = int(time - $start);
        frame($head . $body . "\n$blue$spin[$i % @spin]$off  Working… ${dim}${secs}s$off", 'esc to cancel');
        $i++;
        sleep 0.12;
    }
}

# keys typed or pasted go to the input box; nothing is ever sent
sub typing {
    my ($draw) = @_;
    system('stty', 'raw', '-echo');
    $draw->();
    while (1) {
        my $rin = '';
        vec($rin, fileno(STDIN), 1) = 1;
        my $n = select(my $r = $rin, undef, undef, 0.2);
        if ($n > 0) {
            sysread(STDIN, my $c, 4096) or last;
            utf8::decode($c);
            for my $ch (split //, $c) {
                if ($ch eq "\x7f") { chop $input }
                elsif ($ch eq "\r" || $ch eq "\n") { $input .= "\n" }
                elsif ($ch eq "\x03") { system('stty', 'sane'); exit 0 }
                elsif (ord $ch >= 32) { $input .= $ch }
            }
            $draw->();
        } elsif ($resized) {
            $draw->();
        }
    }
}

if ($role eq 'work') {
    working($steps{work});
} elsif ($role eq 'ask') {
    hook('permission');
    my $menu = "\n${yellow}Requesting permission for:$off\n   ${bold}npm test -- auth$off\nRun this command?\n$blue>$off 1. Yes, run command\n  2. No, cancel\n";
    system('stty', 'raw', '-echo');
    frame($head . $steps{ask} . $menu, 'esc to cancel');
    while (1) {
        my $rin = '';
        vec($rin, fileno(STDIN), 1) = 1;
        my $n = select(my $r = $rin, undef, undef, 0.2);
        if ($n > 0) {
            sysread(STDIN, my $c, 64);
            last if $c =~ /[1\r]/;
        } elsif ($resized) {
            frame($head . $steps{ask} . $menu, 'esc to cancel');
        }
    }
    system('stty', 'sane');
    $start = time;
    working($steps{ask} . step('Run', 'npm test -- auth') . result('12 passed') . step('Edit', 'src/auth/session.ts', diff(3)));
} else {
    hook('done');
    my $answer = "\nThe checkout page shows the cart total now, shipping\nincluded. Committed as ${bold}Show the cart total at checkout$off;\nthe rounding in ${bold}src/cart/total.ts$off waits for your review.\n";
    typing(sub { frame($head . $steps{done} . $answer, '? for shortcuts') });
}
