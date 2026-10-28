
# Sticks Language Manual

Sticks is a strictly evaluated, zero-build template engine. Its core philosophy is that **templates are strings** and **blocks never nest**. 

There is only one lexical primitive: `||` opens a block, and the very next `|` closes it.

## 1. Core Rules
1. **No Nesting:** You cannot open a `||` block inside another `||` block. 
2. **Two Zones:** Execution flows strictly top-to-bottom.
   * **Effect Zone:** Code before the first visible byte of HTML output. You may run database mutations, set cookies, or redirect here.
   * **View Zone:** Begins with the first output byte. It is streamed chunk-by-chunk and is **strictly read-only**. Side effects (`db.exec`, `redirect`) are ignored here.
3. **Chaining:** To chain conditional blocks together, use `|||` (which is simply a closing `|` immediately followed by an opening `||` with no spaces).

---

## 2. Text-Mode Blocks (Control Flow & Output)
Text-mode blocks dictate what HTML is rendered. They consist of a keyword, a condition/iterator, a colon (`:`), and the HTML body.

### Conditional Statements (`if` / `else`)
Use `if` to conditionally render markup. Use chained `||| else:` for the fallback.

```html
|| if $user != null:
    <h1>Welcome back, $user.name!</h1>
||| else:
    <a href="/login">Log in</a>
|
```

### Loops (`for` / `else`)
Iterate over a JSON array. You must declare the type (e.g., `Json`) of the iteration variable.

```html
|| for Json $item in $items:
    <li>$item.name</li>
||| else:
    <li>The list is empty.</li>
|
```
*Note: The `else` block automatically runs if the loop iterates zero times.*

### Interpolation
Inside text-mode bodies (your HTML), you can inject variables using the `$` prefix.
* **Basic:** `$name`, `$user.name`, `$users[0].name`
* **Explicit boundary:** `${user.name}s`
* **URL encoding:** `${url user.profile_link}` (Encodes for safe use inside `href=` or `src=`)
* **Literal Dollar Sign:** Use `$$` to output a single `$`.

---

## 3. Code-Mode Blocks (Statements & Logic)
Code-mode blocks do not output HTML. They execute logic, declare variables, and mutate state. Statements must end with a semicolon `;`.

### Variable Declaration & Assignment
Variables must be declared with a type before use. Valid types in the current engine are `String`, `Int`, `Float`, `Bool`, and `Json`. Variables can hold `null`.

```html
|| String $name = "Alice";
   Json $tasks = [];
   $name = "Bob"; 
|
```

### Type Casting (`as`)
Convert values safely using the `as` keyword:
```html
|| Int $count = $req.query.page as Int;
   String $label = $count as String;
|
```

### Handlers (`on GET`, `on POST`)
Handlers isolate effect-driven logic based on the HTTP method. They **must** be placed in the Effect Zone (before any HTML).
```html
|| on POST:
    if $req.body.action == "delete" {
        db.exec("DELETE FROM items WHERE id = ?", [$req.body.id]);
        redirect("/");
    }
|
```
*Note: Inside code blocks, `if` statements use curly braces `{ }`, unlike text-mode blocks which use colons `:`.*

---

## 4. Built-in Commands & Functions

### Context & Input (`$req`)
The `$req` object is automatically available and contains the request data.
* `$req.method` - HTTP method (`"GET"`, `"POST"`)
* `$req.query` - JSON object of URL query parameters (`?name=Sticks`)
* `$req.body` - JSON object of parsed form data or JSON payloads.

### Database Operations (`db`)
Queries use literal strings with `?` for positional parameters to prevent SQL injection.
* **`db.all(sql, [params])`**: Returns a `Result` object containing a JSON array of rows.
* **`db.exec(sql, [params])`**: Executes a mutation (INSERT, UPDATE, DELETE). *Only allowed in the Effect Zone.*

```html
|| Result $rows = db.all("SELECT * FROM users WHERE role = ?", ["admin"]);
   if $rows.ok == true {
       $users = $rows.value;
   }
|
```

### Network Operations (`http`)
* **`http.get(url, options)`**: Fetches data from an external API.
```html
|| Result $res = http.get("https://api.github.com/repos/merien-io/sticks", {
       headers: { "User-Agent": "sticks" }
   });
|
```

### Session & Cookies (`session`)
* **`session.set(key, value)`**: Sets an HTTP-only cookie.
* **`session.get(key)`**: Reads a cookie value.
* **`session.clear()`**: Clears all session cookies.

### Routing
* **`redirect(path)`**: Halts execution immediately and sends a 303 Redirect to the browser. *Only allowed in the Effect Zone.*

### Utility
* **`len(x)`**: Returns the length of a String, Array, or Object.
* **`not EXPR`**: Inverts a truthy value (`not $is_admin`).

---

## 5. Future Options (Planned, Not Yet Implemented)

The following features are detailed in the Sticks specification and roadmap but are **not yet active or fully implemented** in the current `1.0-draft` Rust engine.

### Data & Network
* **`db.one()`**: A shorthand to return a single row instead of an array.
* **`http.post()`**: For sending outbound mutation requests (currently only `http.get` is implemented).
* **SQLite-Backed Sessions**: `session` currently relies on standard browser cookies. In 2.0, this will transition to opaque IDs linked to a server-side SQLite session table.

### Type System & Validation
* **`input` Blocks**: Currently, the engine skips over `input $form { ... };` blocks. In the future, this will strictly validate `$req.body` and `$req.query` against type/range constraints and populate `$form.errors`.
* **Record Types**: The syntax `type User { String name; }` is currently parsed but bypassed. Custom structural types will eventually enforce rigid database and API schemas.
* **`json(x)` and `${json x}` Helpers**: Planned for safely injecting JSON into `<script>` tags or Alpine.js data attributes without breaking HTML contexts.

### Views & Components
* **`component("path.html", props)`**: Allowing isolated, reusable HTML fragments (like headers and footers) to be included with strict type-checked properties.
* **`<sticks-error>` and `report()`**: A planned mechanism for graceful degradation, where `report($res.error)` will output a standardized HTML tag for frontend frameworks to detect partial failures, rather than crashing the page.
* **Modules (`.stk`)**: Standalone library files containing reusable logic and `pub fn` functions.

### Security & DX (Developer Experience)
* **Handler Hoisting**: Variables declared inside `on POST` are currently scoped only to that block. In the future, they will be hoisted to the file scope so the View Zone can easily render form validation errors.
* **CSRF Helpers**: `csrf.token()` and `csrf.check()` to protect mutations.
* **`status(code)`**: Explicitly setting the HTTP response code (e.g., `status(404)`).
* **Live Reload (`--dev`)**: Scanning and reloading templates on every request during development.