import "./App.css";

function App() {
  return (
    <div className="popover">
      <header className="popover-header">
        <span className="popover-title">Grapevine</span>
      </header>
      <main className="popover-body">
        <p className="placeholder-heading">No pull requests yet</p>
        <p className="placeholder-detail">
          Configure a token and repositories to start watching.
        </p>
      </main>
    </div>
  );
}

export default App;
