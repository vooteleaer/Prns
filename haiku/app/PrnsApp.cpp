/*
 * Prns for Haiku: prnsd's status and controls, with the Haiku API. What the
 * tray icon is on Linux, Windows and macOS: the same icon in the Deskbar,
 * with the same menu (status, the terminal tools, the configuration folder,
 * start and stop), and a window with the interfaces.
 *
 * prnsd runs as the user's launch_daemon job x-vnd.prns-prnsd (the
 * package's data/user_launch, read at login). Right after installing, before
 * the next login, the job isn't known yet: then this app starts prnsd
 * itself. Everything shown comes from `prnsd status --json`.
 */

#include <Alert.h>
#include <Application.h>
#include <Bitmap.h>
#include <Button.h>
#include <Deskbar.h>
#include <Directory.h>
#include <Entry.h>
#include <FindDirectory.h>
#include <LayoutBuilder.h>
#include <ListView.h>
#include <MenuItem.h>
#include <MessageRunner.h>
#include <Path.h>
#include <PopUpMenu.h>
#include <Roster.h>
#include <ScrollView.h>
#include <StringList.h>
#include <StringView.h>
#include <Window.h>
#include <image.h>

#include <Json.h>

#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const char* kSignature = "application/x-vnd.prns";
static const char* kJob = "x-vnd.prns-prnsd";
static const char* kReplicantName = "Prns";
static const char* kTerminal = "application/x-vnd.Haiku-Terminal";
static const char* kTracker = "application/x-vnd.Be-TRAK";

enum {
	kRefresh = 'rfsh',
	kState = 'stat',
	kStart = 'strt',
	kStop = 'stop',
	kRestart = 'rsta',
	kShowStatus = 'shst',
	kInterfaces = 'intf',
	kAnnounce = 'anno',
	kConfigFolder = 'cfgd',
	kOpen = 'open',
	kRemove = 'remo',
	kDone = 'done',
};


// #pragma mark - prnsd


/*! Where the app is: its folder (the package's apps/Prns). */
static BPath
AppFolder()
{
	BPath folder;
	entry_ref ref;
	BPath app;
	if (be_roster->FindApp(kSignature, &ref) == B_OK && app.SetTo(&ref) == B_OK)
		app.GetParent(&folder);
	return folder;
}


/*! prnsd: next to the app in a build folder, else the package's. */
static BString
Prnsd()
{
	BPath p = AppFolder();
	if (p.InitCheck() == B_OK && p.Append("bin/prnsd") == B_OK
		&& access(p.Path(), X_OK) == 0)
		return p.Path();
	return "/boot/system/bin/prnsd";
}


static BString
ConfigDir()
{
	BPath home;
	find_directory(B_USER_DIRECTORY, &home);
	home.Append(".reticulum");
	return home.Path();
}


static BString
Quote(const BString& s)
{
	BString q(s);
	q.ReplaceAll("'", "'\\''");
	return BString("'") << q << "'";
}


/*! Runs a command line, its output in `out`; its exit status. */
static int
Run(const BString& command, BString* out = NULL)
{
	FILE* f = popen(command.String(), "r");
	if (f == NULL)
		return -1;
	char buffer[4096];
	size_t n;
	while ((n = fread(buffer, 1, sizeof(buffer), f)) > 0) {
		if (out != NULL)
			out->Append(buffer, n);
	}
	int status = pclose(f);
	return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}


/*! Runs `launch_roster <action>` on the job; whether it did it. launch_roster
	exits with 0 also when it didn't (an unknown job: "Name not found"),
	saying so only on stderr, so that's what tells: in `error` if given. */
static bool
Roster(const char* action, BString* out, BString* error = NULL)
{
	BString errors = BString("/tmp/prns-roster-") << find_thread(NULL);
	int status = Run(BString("launch_roster ") << action << " " << kJob
		<< " 2>" << errors, out);
	BString message;
	FILE* f = fopen(errors.String(), "r");
	if (f != NULL) {
		char buffer[1024];
		size_t n;
		while ((n = fread(buffer, 1, sizeof(buffer), f)) > 0)
			message.Append(buffer, n);
		fclose(f);
		unlink(errors.String());
	}
	message.Trim();
	if (error != NULL)
		*error = message;
	return status == 0 && message.IsEmpty();
}


/*! Whether the launch_daemon knows the job (not until a login after
	installing), and whether it runs. */
static bool
JobKnown(bool* running)
{
	BString info;
	if (!Roster("info", &info))
		return false;
	*running = info.FindFirst("running = bool(true)") >= 0;
	return true;
}


/*! The teams running prnsd from where this app runs it (Haiku's ps: the
	command line, then team ID, threads, group and user). */
static BStringList
PrnsdTeams()
{
	BStringList teams;
	BString ps;
	Run("ps", &ps);
	BString prnsd = Prnsd();
	BStringList lines;
	ps.Split("\n", true, lines);
	for (int32 i = 0; i < lines.CountStrings(); i++) {
		BString line = lines.StringAt(i);
		line.Trim();
		if (!line.StartsWith(prnsd) || (line.Length() > prnsd.Length()
				&& line[prnsd.Length()] != ' '))
			continue;
		BStringList fields;
		line.Split(" ", true, fields);
		if (fields.CountStrings() >= 5)
			teams.Add(fields.StringAt(fields.CountStrings() - 4));
	}
	return teams;
}


static bool
PrnsdRunning()
{
	// The job's, or one this app ran itself.
	bool running = false;
	if (JobKnown(&running) && running)
		return true;
	return !PrnsdTeams().IsEmpty();
}


/*! Starts prnsd as the launch_daemon's job, or (the job not known yet)
	itself, the first start's configuration included; "" or what failed. */
static BString
StartPrnsd()
{
	bool running = false;
	if (JobKnown(&running)) {
		BString error;
		if (!Roster("start", NULL, &error))
			return error;
		// A job that "starts" and doesn't run (one an older package left,
		// its requirements gone): then prnsd, run here.
		for (int i = 0; i < 10; i++) {
			if (PrnsdRunning())
				return "";
			snooze(500000);
		}
	}
	if (!PrnsdTeams().IsEmpty())
		return "";
	BPath start = AppFolder();
	if (start.InitCheck() != B_OK || start.Append("prnsd-start") != B_OK)
		return "Can't find prnsd-start next to the app.";
	BPath logs;
	find_directory(B_USER_LOG_DIRECTORY, &logs, true);
	BString log = BString(logs.Path()) << "/prnsd.log";
	// In the background, on its own: it stays when this app quits.
	BString command;
	command << "nohup " << Quote(start.Path()) << " >> " << Quote(log)
		<< " 2>&1 < /dev/null &";
	if (Run(command) != 0)
		return "prnsd didn't start.";
	return "";
}


static BString
StopPrnsd()
{
	bool running = false;
	if (JobKnown(&running) && running) {
		BString error;
		if (!Roster("stop", NULL, &error))
			return error;
	}
	// And one this app ran itself, if any.
	BStringList teams = PrnsdTeams();
	for (int32 i = 0; i < teams.CountStrings(); i++)
		kill(atoi(teams.StringAt(i).String()), SIGTERM);
	return "";
}


/*! What there is to show: "running", and from `prnsd status --json` (when
	it answers) "status"; "health" is up, degraded or down. */
static BMessage
ReadState()
{
	BMessage state(kState);
	bool running = PrnsdRunning();
	state.AddBool("running", running);
	BString json;
	BMessage status;
	if (running && Run(BString(Quote(Prnsd())) << " status --json --config "
			<< Quote(ConfigDir()) << " 2>/dev/null", &json) == 0
		&& BJson::Parse(json, status) == B_OK) {
		state.AddMessage("status", &status);
	}
	BString health = "down";
	if (running) {
		health = "up";
		BMessage interfaces;
		if (status.FindMessage("interfaces", &interfaces) == B_OK) {
			for (int32 i = 0; i < interfaces.CountNames(B_ANY_TYPE); i++) {
				BMessage item;
				BString name;
				name << i;
				if (interfaces.FindMessage(name.String(), &item) == B_OK
					&& !item.GetBool("status", false))
					health = "degraded";
			}
		} else
			health = "degraded";
	}
	state.AddString("health", health);
	return state;
}


static BString
Human(double n)
{
	const char* units[] = { "B", "KB", "MB", "GB", "TB" };
	int i = 0;
	while (n >= 1024 && i < 4) {
		n /= 1024;
		i++;
	}
	BString s;
	if (i == 0)
		s.SetToFormat("%.0f B", n);
	else
		s.SetToFormat("%.1f %s", n, units[i]);
	return s;
}


static BMessage
Interfaces(const BMessage& state)
{
	BMessage status, interfaces;
	if (state.FindMessage("status", &status) == B_OK)
		status.FindMessage("interfaces", &interfaces);
	return interfaces;
}


static BMessage
Item(const BMessage& array, int32 i)
{
	BMessage item;
	BString name;
	name << i;
	array.FindMessage(name.String(), &item);
	return item;
}


/*! The tray's status line, as prnsd's own tray says it. */
static BString
StatusLine(const BMessage& state)
{
	if (!state.GetBool("running", false))
		return "prnsd isn't running";
	BMessage interfaces = Interfaces(state);
	int32 count = interfaces.CountNames(B_ANY_TYPE);
	int32 down = 0;
	for (int32 i = 0; i < count; i++) {
		if (!Item(interfaces, i).GetBool("status", false))
			down++;
	}
	BString line;
	if (down > 0)
		line.SetToFormat("Degraded · %d %s unavailable", (int)down,
			down == 1 ? "interface" : "interfaces");
	else
		line.SetToFormat("Running · %d %s", (int)count,
			count == 1 ? "interface" : "interfaces");
	return line;
}


/*! Opens a Terminal running `prnsd <arguments>` in the configuration
	folder, as the tray's terminal actions do; the shell stays after it. */
static void
InTerminal(const char* introduction, const BString& arguments)
{
	BString script;
	script << "clear; printf '%s\\n\\n' " << Quote(introduction) << "; cd "
		<< Quote(ConfigDir()) << " || exit 1; " << Quote(Prnsd()) << " "
		<< arguments << " --config " << Quote(ConfigDir())
		<< "; printf '\\nprnsd command exited with status %s.\\n' \"$?\";"
		<< " exec \"${SHELL:-/bin/sh}\" -l";
	const char* argv[] = { "/bin/sh", "-c", script.String(), NULL };
	be_roster->Launch(kTerminal, 3, argv);
}


static void
OpenConfigFolder()
{
	BString dir = ConfigDir();
	create_directory(dir.String(), 0755);
	entry_ref ref;
	if (get_ref_for_path(dir.String(), &ref) != B_OK)
		return;
	BMessage open(B_REFS_RECEIVED);
	open.AddRef("refs", &ref);
	BMessenger(kTracker).SendMessage(&open);
}


/*! Does a menu or button action (blocking); "" or what went wrong. */
static BString
DoAction(uint32 what)
{
	switch (what) {
		case kStart:
			return StartPrnsd();
		case kStop:
			return StopPrnsd();
		case kRestart:
		{
			BString error = StopPrnsd();
			if (!error.IsEmpty())
				return error;
			for (int i = 0; i < 50 && PrnsdRunning(); i++)
				snooze(100000);
			return StartPrnsd();
		}
		case kShowStatus:
			InTerminal("Reading live Reticulum network status from prnsd.",
				"status");
			break;
		case kInterfaces:
			InTerminal("Opening the guided Reticulum interface editor.",
				"interfaces");
			break;
		case kAnnounce:
			InTerminal("Asking prnsd to announce the hosted NNPages "
				"destination now.", "nnpages announce");
			break;
		case kConfigFolder:
			OpenConfigFolder();
			break;
	}
	return "";
}


// #pragma mark - The icon


/*	Prns's icon, the one prnsd's tray shows on Windows, Linux and macOS:
	prnsd/src/tray/icon.rs rendered the same way (its favicon geometry on a
	100-unit square, 4x4 supersampling), pixel for pixel.
*/

struct Rgba {
	float r, g, b, a;
};

static const Rgba kBackground = { 0x0b, 0x0e, 0x13, 255 };
static const Rgba kMark = { 0x6e, 0xe7, 0xb7, 255 };
static const Rgba kCenter = { 0x34, 0xd3, 0x99, 255 };
static const int kSupersampling = 4;


static float
Channel(float v)
{
	return roundf(std::min(255.0f, std::max(0.0f, v)));
}


static Rgba
Over(Rgba color, Rgba background, float opacity)
{
	float inverse = 1 - opacity;
	return { Channel(color.r * opacity + background.r * inverse),
		Channel(color.g * opacity + background.g * inverse),
		Channel(color.b * opacity + background.b * inverse), 255 };
}


static bool
InsideRoundedSquare(float x, float y)
{
	float nx = std::min(78.0f, std::max(22.0f, x));
	float ny = std::min(78.0f, std::max(22.0f, y));
	return hypotf(x - nx, y - ny) <= 22.0f;
}


static float
DistanceToSegment(float px, float py, float sx, float sy, float ex, float ey)
{
	float dx = ex - sx, dy = ey - sy;
	float t = ((px - sx) * dx + (py - sy) * dy) / (dx * dx + dy * dy);
	t = std::min(1.0f, std::max(0.0f, t));
	return hypotf(px - (sx + t * dx), py - (sy + t * dy));
}


static bool
OnCardinalTick(float x, float y)
{
	float radians = -46.0f * (float)M_PI / 180.0f;
	float dx = x - 50, dy = y - 50;
	float ux = 50 + dx * cosf(radians) - dy * sinf(radians);
	float uy = 50 + dx * sinf(radians) + dy * cosf(radians);
	const float ticks[4][4] = {
		{ 50, 7, 50, 16 }, { 50, 84, 50, 93 }, { 7, 50, 16, 50 },
		{ 84, 50, 93, 50 } };
	for (int i = 0; i < 4; i++) {
		if (DistanceToSegment(ux, uy, ticks[i][0], ticks[i][1], ticks[i][2],
				ticks[i][3]) <= 1.7f)
			return true;
	}
	return false;
}


static bool
OnRadioArc(float dx, float dy, float radius, float halfWidth)
{
	float angle = atan2f(dy, dx) * 180.0f / (float)M_PI;
	return fabsf(hypotf(dx, dy) - radius) <= halfWidth
		&& (fabsf(angle) <= 55.0f || fabsf(angle) >= 125.0f);
}


static Rgba
FaviconSample(float x, float y)
{
	if (!InsideRoundedSquare(x, y))
		return { 0, 0, 0, 0 };
	Rgba color = kBackground;
	float mx = (x - 13.0f) / 0.74f;
	float my = (y - 13.0f) / 0.74f;
	float dx = mx - 50, dy = my - 50;
	float radius = hypotf(dx, dy);
	if (fabsf(radius - 37.0f) <= 1.7f || OnCardinalTick(mx, my))
		color = kMark;
	if (OnRadioArc(dx, dy, 13.0f, 1.5f))
		color = Over(kMark, color, 0.9f);
	if (OnRadioArc(dx, dy, 21.0f, 1.5f))
		color = Over(kMark, color, 0.45f);
	if (radius <= 6.5f)
		color = kCenter;
	return color;
}


/*! The icon at `size` pixels, as a B_RGBA32 bitmap (alpha straight). */
static BBitmap*
RenderIcon(int32 size)
{
	BBitmap* bitmap = new BBitmap(BRect(0, 0, size - 1, size - 1), B_RGBA32);
	uint8* bits = (uint8*)bitmap->Bits();
	int32 bpr = bitmap->BytesPerRow();
	for (int32 py = 0; py < size; py++) {
		for (int32 px = 0; px < size; px++) {
			float sum[3] = { 0, 0, 0 };
			float alpha = 0;
			for (int sy = 0; sy < kSupersampling; sy++) {
				for (int sx = 0; sx < kSupersampling; sx++) {
					float x = (px + (sx + 0.5f) / kSupersampling) * 100.0f / size;
					float y = (py + (sy + 0.5f) / kSupersampling) * 100.0f / size;
					Rgba s = FaviconSample(x, y);
					float a = s.a / 255.0f;
					sum[0] += s.r * a;
					sum[1] += s.g * a;
					sum[2] += s.b * a;
					alpha += a;
				}
			}
			uint8* p = bits + py * bpr + px * 4;
			if (alpha == 0) {
				p[0] = p[1] = p[2] = p[3] = 0;
				continue;
			}
			float samples = kSupersampling * kSupersampling;
			// B_RGBA32 is B, G, R, A in memory.
			p[0] = (uint8)Channel(sum[2] / alpha);
			p[1] = (uint8)Channel(sum[1] / alpha);
			p[2] = (uint8)Channel(sum[0] / alpha);
			p[3] = (uint8)Channel(alpha / samples * 255.0f);
		}
	}
	return bitmap;
}


/*! Draws the icon centred in `view`'s bounds, rendered for their size. */
static void
DrawIcon(BView* view, BBitmap*& icon)
{
	BRect bounds = view->Bounds();
	int32 size = (int32)std::min(bounds.Width(), bounds.Height()) + 1;
	if (icon == NULL || icon->Bounds().IntegerWidth() + 1 != size) {
		delete icon;
		icon = RenderIcon(size);
	}
	view->PushState();
	view->SetDrawingMode(B_OP_ALPHA);
	view->SetBlendingMode(B_PIXEL_ALPHA, B_ALPHA_OVERLAY);
	view->DrawBitmap(icon, BPoint((bounds.Width() + 1 - size) / 2,
		(bounds.Height() + 1 - size) / 2));
	view->PopState();
}


// #pragma mark - Window


class InterfaceItem : public BListItem {
public:
	InterfaceItem(const BMessage& item)
		:
		fUp(item.GetBool("status", false)),
		fName(item.GetString("name", "?"))
	{
		fDetails << item.GetString("type", "") << " · "
			<< (fUp ? "up" : "down") << " · ↓ " << Human(item.GetDouble("rxb", 0))
			<< "  ↑ " << Human(item.GetDouble("txb", 0));
	}

	void DrawItem(BView* owner, BRect frame, bool complete)
	{
		owner->SetLowColor(IsSelected()
			? ui_color(B_LIST_SELECTED_BACKGROUND_COLOR)
			: ui_color(B_LIST_BACKGROUND_COLOR));
		owner->FillRect(frame, B_SOLID_LOW);
		font_height fh;
		owner->GetFontHeight(&fh);
		float y = frame.top + fh.ascent + 4;
		owner->SetHighColor(fUp ? make_color(0x34, 0xa8, 0x53)
			: make_color(0xd9, 0x30, 0x25));
		owner->FillEllipse(BPoint(frame.left + 10, y - fh.ascent / 2 + 1), 4, 4);
		owner->SetHighColor(ui_color(B_LIST_ITEM_TEXT_COLOR));
		owner->DrawString(fName, BPoint(frame.left + 22, y));
		owner->SetHighColor(tint_color(ui_color(B_LIST_ITEM_TEXT_COLOR),
			B_LIGHTEN_1_TINT));
		owner->DrawString(fDetails, BPoint(frame.left + 22,
			y + fh.ascent + fh.descent + fh.leading + 2));
	}

	void Update(BView* owner, const BFont* font)
	{
		BListItem::Update(owner, font);
		font_height fh;
		font->GetHeight(&fh);
		SetHeight(2 * (fh.ascent + fh.descent + fh.leading) + 10);
	}

private:
	bool fUp;
	BString fName;
	BString fDetails;
};


class IconView : public BView {
public:
	IconView()
		:
		BView("icon", B_WILL_DRAW),
		fIcon(NULL)
	{
		SetExplicitSize(BSize(47, 47));
	}

	~IconView()
	{
		delete fIcon;
	}

	void AttachedToWindow()
	{
		AdoptParentColors();
	}

	void Draw(BRect)
	{
		DrawIcon(this, fIcon);
	}

private:
	BBitmap* fIcon;
};


/*! Reads the state and does actions off the window's thread. */
class Worker : public BLooper {
public:
	Worker(BMessenger target)
		:
		BLooper("prns worker"),
		fTarget(target)
	{
	}

	void MessageReceived(BMessage* message)
	{
		if (message->what == kRefresh) {
			BMessage state = ReadState();
			fTarget.SendMessage(&state);
			return;
		}
		BMessage done(kDone);
		done.AddString("error", DoAction(message->what));
		fTarget.SendMessage(&done);
		PostMessage(kRefresh);
	}

private:
	BMessenger fTarget;
};


class PrnsWindow : public BWindow {
public:
	PrnsWindow()
		:
		BWindow(BRect(120, 120, 600, 520), "Prns", B_TITLED_WINDOW,
			B_AUTO_UPDATE_SIZE_LIMITS | B_QUIT_ON_WINDOW_CLOSE)
	{
		fIcon = new IconView();
		fHeadline = new BStringView("headline", "Reading prnsd's status…");
		BFont bold(be_bold_font);
		bold.SetSize(bold.Size() * 1.2f);
		fHeadline->SetFont(&bold);
		fDetails = new BStringView("details", "");
		fTraffic = new BStringView("traffic", "");
		fList = new BListView("interfaces");
		fStart = new BButton("Start", new BMessage(kStart));
		fStop = new BButton("Stop", new BMessage(kStop));
		fRestart = new BButton("Restart", new BMessage(kRestart));

		BLayoutBuilder::Group<>(this, B_VERTICAL)
			.SetInsets(B_USE_WINDOW_SPACING)
			.AddGroup(B_HORIZONTAL)
				.Add(fIcon)
				.AddGroup(B_VERTICAL, 2)
					.Add(fHeadline)
					.Add(fDetails)
					.Add(fTraffic)
				.End()
				.AddGlue()
			.End()
			.Add(new BStringView("label", "Interfaces"))
			.Add(new BScrollView("scroll", fList, 0, false, true))
			.AddGroup(B_HORIZONTAL)
				.Add(fStart)
				.Add(fStop)
				.Add(fRestart)
				.AddGlue()
				.Add(new BButton("Interfaces" B_UTF8_ELLIPSIS,
					new BMessage(kInterfaces)))
				.Add(new BButton("Configuration folder",
					new BMessage(kConfigFolder)))
			.End();
		fList->SetExplicitMinSize(BSize(420, 200));

		fWorker = new Worker(BMessenger(this));
		fWorker->Run();
		fWorker->PostMessage(kRefresh);
		BMessage refresh(kRefresh);
		fRunner = new BMessageRunner(BMessenger(fWorker), &refresh, 3000000);
	}

	~PrnsWindow()
	{
		delete fRunner;
		fWorker->Lock();
		fWorker->Quit();
	}

	void MessageReceived(BMessage* message)
	{
		switch (message->what) {
			case kState:
				_Show(*message);
				break;
			case kDone:
			{
				BString error = message->GetString("error", "");
				if (!error.IsEmpty())
					(new BAlert("Prns", error, "OK"))->Go(NULL);
				break;
			}
			case kStart:
			case kStop:
			case kRestart:
			case kInterfaces:
			case kConfigFolder:
				fWorker->PostMessage(message->what);
				break;
			default:
				BWindow::MessageReceived(message);
		}
	}

private:
	void _Show(const BMessage& state)
	{
		bool running = state.GetBool("running", false);
		fHeadline->SetText(StatusLine(state));
		BMessage status;
		if (state.FindMessage("status", &status) == B_OK) {
			BString details, traffic;
			int64 up = (int64)status.GetDouble("transport_uptime", 0);
			details.SetToFormat("Up %dd %02d:%02d · transport %s",
				(int)(up / 86400), (int)(up / 3600 % 24), (int)(up / 60 % 60),
				status.GetString("transport_id", "?"));
			traffic << "Received " << Human(status.GetDouble("rxb", 0))
				<< " · sent " << Human(status.GetDouble("txb", 0));
			fDetails->SetText(details);
			fTraffic->SetText(traffic);
		} else {
			fDetails->SetText(running ? "Not answering yet" B_UTF8_ELLIPSIS
				: "Start it here, or log in again to have it start by itself.");
			fTraffic->SetText("");
		}
		fList->MakeEmpty();
		BMessage interfaces = Interfaces(state);
		for (int32 i = 0; i < interfaces.CountNames(B_ANY_TYPE); i++)
			fList->AddItem(new InterfaceItem(Item(interfaces, i)));
		fStart->SetEnabled(!running);
		fStop->SetEnabled(running);
		fRestart->SetEnabled(running);
	}

	IconView* fIcon;
	BStringView* fHeadline;
	BStringView* fDetails;
	BStringView* fTraffic;
	BListView* fList;
	BButton* fStart;
	BButton* fStop;
	BButton* fRestart;
	Worker* fWorker;
	BMessageRunner* fRunner;
};


// #pragma mark - Deskbar replicant


/*!	Prns in the Deskbar's tray, with the tray menu of prnsd's own. Deskbar
	loads the app's image and calls instantiate_deskbar_item(); the view lives
	in the Deskbar's process, so reading prnsd's state runs in a thread of
	its own.
*/
/*!	Keeps this image loaded in the Deskbar's process for good: a replicant's
	image unloaded (a new version replacing it) left behind something a
	later fork() in the Deskbar jumped into, and the Deskbar crashed. A
	second reference, never dropped, keeps it mapped.
*/
static void
PinImage()
{
	static bool pinned = false;
	if (pinned)
		return;
	pinned = true;
	addr_t self = (addr_t)&PinImage;
	image_info info;
	int32 cookie = 0;
	while (get_next_image_info(B_CURRENT_TEAM, &cookie, &info) == B_OK) {
		addr_t start = (addr_t)info.text;
		if (self >= start && self < start + info.text_size) {
			load_add_on(info.name);
			return;
		}
	}
}


class _EXPORT DeskbarView : public BView {
public:
	DeskbarView(BRect frame)
		:
		BView(frame, kReplicantName, B_FOLLOW_NONE, B_WILL_DRAW),
		fIcon(NULL),
		fPoller(-1),
		fQuit(false)
	{
	}

	DeskbarView(BMessage* archive)
		:
		BView(archive),
		fIcon(NULL),
		fPoller(-1),
		fQuit(false)
	{
	}

	~DeskbarView()
	{
		delete fIcon;
	}

	// Defined outside the class: Deskbar looks Instantiate() up by name.
	static BArchivable* Instantiate(BMessage* archive);
	status_t Archive(BMessage* data, bool deep = true) const;

	void AttachedToWindow()
	{
		BView::AttachedToWindow();
		PinImage();
		AdoptParentColors();
		fPoller = spawn_thread(_Poll, "prns tray", B_LOW_PRIORITY, this);
		resume_thread(fPoller);
	}

	void DetachedFromWindow()
	{
		fQuit = true;
		status_t ignored;
		wait_for_thread(fPoller, &ignored);
		BView::DetachedFromWindow();
	}

	void Draw(BRect)
	{
		DrawIcon(this, fIcon);
	}

	void MessageReceived(BMessage* message)
	{
		switch (message->what) {
			case kState:
				fRunning = message->GetBool("running", false);
				fLine = StatusLine(*message);
				break;
			case kOpen:
				be_roster->Launch(kSignature);
				break;
			case kRemove:
				_Detached(_Remove, 0);
				break;
			case kStart:
			case kStop:
			case kShowStatus:
			case kInterfaces:
			case kAnnounce:
			case kConfigFolder:
				_Detached(_Perform, message->what);
				break;
			default:
				BView::MessageReceived(message);
		}
	}

	void MouseDown(BPoint where)
	{
		BPopUpMenu* menu = new BPopUpMenu("Prns", false, false);
		BMenuItem* title = new BMenuItem(fVersion.IsEmpty()
			? "Prns Daemon" : fVersion.String(), NULL);
		title->SetEnabled(false);
		menu->AddItem(title);
		BMenuItem* line = new BMenuItem(fLine.IsEmpty()
			? "Reading prnsd's status…" : fLine.String(), NULL);
		line->SetEnabled(false);
		menu->AddItem(line);
		menu->AddSeparatorItem();
		menu->AddItem(new BMenuItem("Open Prns" B_UTF8_ELLIPSIS,
			new BMessage(kOpen)));
		menu->AddItem(new BMenuItem("Show Network Status",
			new BMessage(kShowStatus)));
		menu->AddItem(new BMenuItem("Announce NNPages Now",
			new BMessage(kAnnounce)));
		menu->AddItem(new BMenuItem("Manage Interfaces" B_UTF8_ELLIPSIS,
			new BMessage(kInterfaces)));
		menu->AddItem(new BMenuItem("Open Configuration Folder",
			new BMessage(kConfigFolder)));
		menu->AddSeparatorItem();
		menu->AddItem(fRunning
			? new BMenuItem("Stop prnsd", new BMessage(kStop))
			: new BMenuItem("Start prnsd", new BMessage(kStart)));
		menu->AddItem(new BMenuItem("Remove from Deskbar",
			new BMessage(kRemove)));
		menu->SetTargetForItems(this);
		menu->SetAsyncAutoDestruct(true);
		menu->Go(ConvertToScreen(where), true, true, true);
	}

private:
	static status_t _Poll(void* data)
	{
		DeskbarView* view = (DeskbarView*)data;
		BMessenger target(view);
		BString version;
		if (Run(BString(Quote(Prnsd())) << " --version 2>/dev/null", &version)
				== 0) {
			version.Trim();
			version.ReplaceFirst("prnsd ", "Prns Daemon · v");
			view->fVersion = version;
		}
		while (!view->fQuit) {
			BMessage state = ReadState();
			target.SendMessage(&state);
			for (int i = 0; i < 50 && !view->fQuit; i++)
				snooze(100000);
		}
		return B_OK;
	}

	static void _Detached(thread_func f, uint32 what)
	{
		thread_id t = spawn_thread(f, "prns tray action", B_NORMAL_PRIORITY,
			(void*)(addr_t)what);
		resume_thread(t);
	}

	static status_t _Perform(void* data)
	{
		BString error = DoAction((uint32)(addr_t)data);
		if (!error.IsEmpty())
			(new BAlert("Prns", error, "OK"))->Go(NULL);
		return B_OK;
	}

	static status_t _Remove(void*)
	{
		BDeskbar().RemoveItem(kReplicantName);
		return B_OK;
	}

	BBitmap* fIcon;
	BString fLine;
	BString fVersion;
	bool fRunning = false;
	thread_id fPoller;
	volatile bool fQuit;
};


BArchivable*
DeskbarView::Instantiate(BMessage* archive)
{
	if (!validate_instantiation(archive, "DeskbarView"))
		return NULL;
	return new DeskbarView(archive);
}


status_t
DeskbarView::Archive(BMessage* data, bool deep) const
{
	status_t status = BView::Archive(data, deep);
	if (status == B_OK)
		status = data->AddString("add_on", kSignature);
	if (status == B_OK)
		status = data->AddString("class", "DeskbarView");
	return status;
}


extern "C" _EXPORT BView*
instantiate_deskbar_item(float maxWidth, float maxHeight)
{
	return new DeskbarView(BRect(0, 0, maxHeight - 1, maxHeight - 1));
}


/*!	Puts the icon in the Deskbar, unless it is there already (Deskbar keeps
	it across restarts).
*/
static void
InstallInDeskbar()
{
	BDeskbar deskbar;
	// At login (the package's launch job) the Deskbar may not be up yet.
	for (int i = 0; i < 30 && !deskbar.IsRunning(); i++)
		snooze(1000000);
	if (!deskbar.IsRunning() || deskbar.HasItem(kReplicantName))
		return;
	app_info info;
	if (be_app->GetAppInfo(&info) == B_OK)
		deskbar.AddItem(&info.ref);
}


// #pragma mark - App


class PrnsApp : public BApplication {
public:
	PrnsApp()
		:
		BApplication(kSignature),
		fDeskbarOnly(false)
	{
	}

	void ArgvReceived(int32 argc, char** argv)
	{
		for (int32 i = 1; i < argc; i++) {
			if (strcmp(argv[i], "--deskbar") == 0)
				fDeskbarOnly = true;
		}
	}

	void ReadyToRun()
	{
		InstallInDeskbar();
		if (fDeskbarOnly) {
			Quit();
			return;
		}
		(new PrnsWindow())->Show();
	}

private:
	bool fDeskbarOnly;
};


int
main()
{
	PrnsApp app;
	app.Run();
	return 0;
}
