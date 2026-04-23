pub const ZSH: &str = r#"#compdef mjira

_mjira_issues() {
  local -a keys descs
  local k d
  while IFS=$'\t' read -r k d; do
    keys+=("$k")
    descs+=("$k -- $d")
  done < <(mjira _complete-issues 2>/dev/null)
  (( ${#keys} )) || return 1
  compadd -l -d descs -a keys
}

_mjira_instances() {
  local -a names
  names=(${(f)"$(mjira _complete-instances 2>/dev/null)"})
  (( ${#names} )) || return 1
  compadd -a names
}

_mjira_columns() {
  _values -s , 'column' \
    'key[Issue key]' \
    'type[Issue type]' \
    'status[Status]' \
    'assignee[Assignee]' \
    'priority[Priority]' \
    'updated[Last updated]' \
    'summary[Summary]' \
    'project[Project]' \
    'parent[Parent issue]' \
    'components[Components]' \
    'labels[Labels]'
}

_mjira_issue() {
  local context state state_descr line
  typeset -A opt_args

  _arguments -C \
    '1: :->sub' \
    '*:: :->args'

  case $state in
    sub)
      local -a subs
      subs=(
        'list:List issues'
        'get:Show issue details'
        'create:Create a new issue'
        'comment:Add a comment'
        'transition:Change issue status'
        'assign:Assign an issue'
        'values:List valid field values'
        'commits:Find git commits for an issue'
        'diff:Show git diff for an issue'
      )
      _describe 'subcommand' subs
      ;;
    args)
      case $line[1] in
        get)
          _arguments \
            '--images[Display image attachments via kitty icat]' \
            '1:issue key:_mjira_issues'
          ;;
        comment)
          _arguments \
            '1:issue key:_mjira_issues' \
            '2:body:'
          ;;
        transition)
          _arguments \
            '1:issue key:_mjira_issues' \
            '2::status:' \
            '--assign=[Assign after transition]:user:' \
            '--unassign[Unassign after transition]' \
            '--transition-parent[Also transition parent issue]'
          ;;
        assign)
          _arguments \
            '1:issue key:_mjira_issues' \
            '2:assignee:'
          ;;
        commits)
          _arguments \
            '1:issue key:_mjira_issues' \
            '*-r[Additional repo path]:repo:_files -/' \
            '*--repo[Additional repo path]:repo:_files -/' \
            '--verbose[Show repos with no results]'
          ;;
        diff)
          _arguments \
            '1:issue key:_mjira_issues' \
            '(-c --commit)'{-c,--commit}'[Specific commit hash]:commit:' \
            '*-r[Additional repo path]:repo:_files -/' \
            '*--repo[Additional repo path]:repo:_files -/' \
            '--verbose[Show repos with no results]'
          ;;
        list)
          _arguments \
            '(-p --project)'{-p,--project}'[Filter by project]:project:' \
            '(-a --assignee)'{-a,--assignee}'[Filter by assignee]:assignee:' \
            '--any-assignee[Remove assignee filter]' \
            '(-s --status)'{-s,--status}'[Filter by status]:status:' \
            '(-t --type)'{-t,--type}'[Filter by issue type]:type:' \
            '--jql[Extra JQL]:jql:' \
            '(-l --limit)'{-l,--limit}'[Max results]:limit:' \
            '(-c --columns)'{-c,--columns}'[Columns]:columns:_mjira_columns' \
            '--list-columns[Print available columns]'
          ;;
        create)
          _arguments \
            '(-p --project)'{-p,--project}'[Project key]:project:' \
            '(-s --summary)'{-s,--summary}'[Summary]:summary:' \
            '(-t --type)'{-t,--type}'[Issue type]:type:' \
            '(-d --description)'{-d,--description}'[Description]:description:' \
            '--priority[Priority]:priority:' \
            '(-a --assignee)'{-a,--assignee}'[Assignee]:assignee:'
          ;;
        values)
          _arguments \
            '1:field:' \
            '(-p --project)'{-p,--project}'[Project key]:project:'
          ;;
      esac
      ;;
  esac
}

_mjira_instance() {
  local context state state_descr line
  typeset -A opt_args

  _arguments -C \
    '1: :->sub' \
    '*:: :->args'

  case $state in
    sub)
      local -a subs
      subs=(
        'list:List configured instances'
        'add:Add a new instance'
        'remove:Remove an instance'
        'set-default:Set the default instance'
        'path:Print config file path'
      )
      _describe 'subcommand' subs
      ;;
    args)
      case $line[1] in
        add)
          _arguments \
            '1:name:' \
            '--url[Base URL]:url:' \
            '--username[Username]:username:' \
            '--api-key[API token]:key:' \
            '--password[Password]:password:' \
            '--api-version[API version (2 or 3)]:version:(2 3)'
          ;;
        remove|set-default)
          _arguments '1:instance:_mjira_instances'
          ;;
      esac
      ;;
  esac
}

_mjira_board() {
  local context state state_descr line
  typeset -A opt_args

  _arguments -C \
    '1: :->sub' \
    '*:: :->args'

  case $state in
    sub)
      local -a subs
      subs=(
        'list:List boards'
        'issues:List issues on a board'
        'quick-filters:List quick filters for a board'
      )
      _describe 'subcommand' subs
      ;;
    args)
      case $line[1] in
        list)
          _arguments \
            '(-p --project)'{-p,--project}'[Filter by project]:project:' \
            '(-n --name)'{-n,--name}'[Filter by board name]:name:' \
            '(-l --limit)'{-l,--limit}'[Max boards]:limit:'
          ;;
        issues)
          _arguments \
            '1:board ID:' \
            '(-l --limit)'{-l,--limit}'[Max results]:limit:' \
            '(-c --columns)'{-c,--columns}'[Columns]:columns:_mjira_columns' \
            '(-j --jql)'{-j,--jql}'[Extra JQL]:jql:' \
            '(-q --quick-filter)'{-q,--quick-filter}'[Quick filter ID]:id:'
          ;;
        quick-filters)
          _arguments '1:board ID:'
          ;;
      esac
      ;;
  esac
}

_mjira_project() {
  local context state state_descr line
  typeset -A opt_args

  _arguments -C \
    '1: :->sub' \
    '*:: :->args'

  case $state in
    sub)
      local -a subs
      subs=('list:List projects')
      _describe 'subcommand' subs
      ;;
    args)
      case $line[1] in
        list)
          _arguments '(-q --query)'{-q,--query}'[Search by name/key]:query:'
          ;;
      esac
      ;;
  esac
}

_mjira() {
  local context state state_descr line
  typeset -A opt_args

  _arguments -C \
    '(-i --instance)'{-i,--instance}'[Instance alias]:instance:_mjira_instances' \
    '(-v --verbose)'{-v,--verbose}'[Print HTTP requests to stderr]' \
    '--very-verbose[Print full request/response bodies]' \
    '1: :->command' \
    '*:: :->args'

  case $state in
    command)
      local -a cmds
      cmds=(
        'instance:Manage configured Jira instances'
        'issue:Work with Jira issues'
        'project:Work with Jira projects'
        'board:Work with Jira boards'
        'search:Search issues using JQL'
        'query:Run a saved query from config'
      )
      _describe 'command' cmds
      ;;
    args)
      case $line[1] in
        issue)    _mjira_issue ;;
        instance) _mjira_instance ;;
        board)    _mjira_board ;;
        project)  _mjira_project ;;
        search)
          _arguments \
            '1::jql:' \
            '(-l --limit)'{-l,--limit}'[Max results]:limit:' \
            '(-c --columns)'{-c,--columns}'[Columns]:columns:_mjira_columns' \
            '--list-columns[Print available columns]'
          ;;
        query)
          _arguments \
            '1::query name:' \
            '(-l --limit)'{-l,--limit}'[Max results]:limit:' \
            '(-c --columns)'{-c,--columns}'[Columns]:columns:_mjira_columns' \
            '--list-columns[Print available columns]'
          ;;
      esac
      ;;
  esac
}

compdef _mjira mjira
"#;
